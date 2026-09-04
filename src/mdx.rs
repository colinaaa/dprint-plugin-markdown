//! MDX support via pre/post-processing.
//!
//! Uses `markdown-rs` (the reference MDX parser) to locate every MDX-specific
//! node, protects each one with a placeholder while the markdown formatter
//! runs, then restores the (optionally formatted) originals.
//!
//! Block-level ESM (`import`/`export`) and flow expressions / JSX are offered
//! to the host's TypeScript formatter via the code-block callback (tag `tsx`).
//! If the host has `@dprint/typescript` installed it will format them;
//! otherwise the original text is kept.

use dprint_core::configuration::resolve_new_line_kind;

use crate::configuration::Configuration;
use crate::format_text::FormatError;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Formats an MDX file.
pub fn format_mdx_text(
  file_text: &str,
  config: &Configuration,
  mut format_code_block_text: impl for<'a> FnMut(&str, &'a str, u32) -> Result<Option<String>, FormatError>,
) -> Result<Option<String>, FormatError> {
  let new_line = resolve_new_line_kind(file_text, config.new_line_kind);

  // 1. Strip frontmatter: the repository's parser accepts `...` as YAML
  //    closer but markdown-rs does not. Strip it so markdown-rs only sees
  //    the body.
  let (frontmatter, body) = split_frontmatter(file_text);

  // 2. Parse the body with markdown-rs in MDX mode.
  let tree = match parse_mdx_tree(body) {
    Some(tree) => tree,
    None => return Ok(None), // unparseable MDX → return unchanged
  };

  // 3. Collect MDX spans (byte offsets into `body`).
  let mut spans = Vec::new();
  collect_mdx_spans(&tree, &mut spans);
  spans.sort_by_key(|s| s.start);
  let spans = merge_spans(spans);

  if spans.is_empty() {
    return crate::format_text(file_text, config, format_code_block_text);
  }

  // 4. For each span, try to format via the tsx callback, build a unique
  //    placeholder that preserves the original newline count.
  let nonce = find_nonce(file_text);
  let line_width = config.line_width;
  let regions: Vec<MdxRegion> = spans
    .iter()
    .enumerate()
    .map(|(i, span)| {
      let original = &body[span.start..span.end];

      // Try formatting block-level MDX via the host's TypeScript formatter.
      let restored = if span.formattable {
        try_format_region(original, line_width, &mut format_code_block_text)
      } else {
        original.to_string()
      };
      let restored = normalize_newlines(&restored, new_line);

      // Placeholder: HTML comment. Padding newlines are appended to the
      // placeholdered text (not part of the placeholder itself) so the
      // formatter can normalize them without breaking restoration.
      let newlines = original.chars().filter(|c| *c == '\n').count();
      let placeholder = if let Some(directive) = as_ignore_directive(original, config) {
        format!("<!-- {} -->", directive)
      } else {
        format!("<!-- {} {} -->", nonce, i)
      };

      MdxRegion { placeholder, restored, newline_padding: newlines }
    })
    .collect();

  // 5. Assemble placeholdered text.
  let mut placeholdered = String::with_capacity(file_text.len());
  if let Some(fm) = frontmatter {
    placeholdered.push_str(fm);
  }
  let mut pos = 0;
  for (span, region) in spans.iter().zip(regions.iter()) {
    placeholdered.push_str(&body[pos..span.start]);
    placeholdered.push_str(&region.placeholder);
    for _ in 0..region.newline_padding {
      placeholdered.push('\n');
    }
    pos = span.end;
  }
  placeholdered.push_str(&body[pos..]);

  // 6. Format the placeholdered text as plain markdown.
  let formatted = crate::format_text(&placeholdered, config, format_code_block_text)?;
  let formatted_text = formatted.as_deref().unwrap_or(&placeholdered);

  // 7. Restore regions in a single forward scan.
  let result = restore_regions(formatted_text, &regions);

  if result == file_text {
    Ok(None)
  } else {
    Ok(Some(result))
  }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

struct MdxSpan {
  start: usize,
  end: usize,
  /// Whether this region can be sent to the TypeScript formatter.
  formattable: bool,
}

struct MdxRegion {
  placeholder: String,
  /// What to put back: either formatted output or the original.
  restored: String,
  /// Number of newlines to add after the placeholder in the placeholdered text
  /// (to preserve line count for diagnostics).
  newline_padding: usize,
}

// ---------------------------------------------------------------------------
// Frontmatter
// ---------------------------------------------------------------------------

fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
  let first_line = text.lines().next().unwrap_or("");
  let close_markers: &[&str] = match first_line.trim_end() {
    "---" => &["---", "..."],
    "+++" => &["+++"],
    _ => return (None, text),
  };
  // Advance past the first line (including its newline).
  let mut offset = first_line.len();
  offset += line_ending_len(text, offset);
  if offset >= text.len() {
    return (None, text);
  }
  // Second line must exist, be non-blank, and not be a closer.
  let second_line = text[offset..].lines().next().unwrap_or("");
  if second_line.trim().is_empty() || close_markers.contains(&second_line.trim_end()) {
    return (None, text);
  }
  // Advance past the second line.
  offset += second_line.len();
  offset += line_ending_len(text, offset);
  // Scan remaining lines for the closing marker.
  while offset < text.len() {
    let line = text[offset..].lines().next().unwrap_or("");
    offset += line.len();
    offset += line_ending_len(text, offset);
    if close_markers.contains(&line.trim_end()) {
      return (Some(&text[..offset]), &text[offset..]);
    }
  }
  (None, text) // unclosed
}

/// Length of the line ending at `offset` (0, 1, or 2).
fn line_ending_len(text: &str, offset: usize) -> usize {
  if text[offset..].starts_with("\r\n") {
    2
  } else if text[offset..].starts_with('\n') {
    1
  } else {
    0
  }
}

// ---------------------------------------------------------------------------
// MDX parsing
// ---------------------------------------------------------------------------

fn parse_mdx_tree(source: &str) -> Option<markdown::mdast::Node> {
  let opts = markdown::ParseOptions {
    constructs: markdown::Constructs {
      frontmatter: true,
      ..markdown::Constructs::mdx()
    },
    mdx_esm_parse: Some(Box::new(esm_parse)),
    mdx_expression_parse: Some(Box::new(expression_parse)),
    ..markdown::ParseOptions::mdx()
  };
  std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| markdown::to_mdast(source, &opts)))
    .ok()?
    .ok()
}

fn esm_parse(value: &str) -> markdown::MdxSignal {
  if js_braces_balanced(value) {
    markdown::MdxSignal::Ok
  } else {
    markdown::MdxSignal::Eof("Incomplete ESM".into(), Box::default(), Box::default())
  }
}

fn expression_parse(value: &str, _kind: &markdown::MdxExpressionKind) -> markdown::MdxSignal {
  if js_braces_balanced(value) {
    markdown::MdxSignal::Ok
  } else {
    markdown::MdxSignal::Eof(
      "Incomplete expression".into(),
      Box::default(),
      Box::default(),
    )
  }
}

/// Checks brace balance, skipping contents of strings, template literals,
/// single-line comments (`//`), and block comments (`/* */`).
fn js_braces_balanced(value: &str) -> bool {
  let b = value.as_bytes();
  let len = b.len();
  let mut depth: i32 = 0;
  let mut i = 0;
  while i < len {
    match b[i] {
      b'\'' | b'"' => {
        let q = b[i];
        i += 1;
        while i < len && b[i] != q {
          if b[i] == b'\\' {
            i += 1;
          }
          i += 1;
        }
        i += 1;
      }
      b'`' => {
        i += 1;
        while i < len && b[i] != b'`' {
          if b[i] == b'\\' {
            i += 1;
          } else if b[i] == b'$' && i + 1 < len && b[i + 1] == b'{' {
            i += 2;
            let mut nest = 1i32;
            while i < len && nest > 0 {
              match b[i] {
                b'{' => nest += 1,
                b'}' => nest -= 1,
                b'\\' => {
                  i += 1;
                }
                _ => {}
              }
              i += 1;
            }
            continue;
          }
          i += 1;
        }
        i += 1;
      }
      b'/' if i + 1 < len && b[i + 1] == b'/' => {
        i += 2;
        while i < len && b[i] != b'\n' {
          i += 1;
        }
      }
      b'/' if i + 1 < len && b[i + 1] == b'*' => {
        i += 2;
        while i + 1 < len && !(b[i] == b'*' && b[i + 1] == b'/') {
          i += 1;
        }
        i += 2;
      }
      b'{' => {
        depth += 1;
        i += 1;
      }
      b'}' => {
        depth -= 1;
        i += 1;
      }
      _ => {
        i += 1;
      }
    }
  }
  depth <= 0
}

// ---------------------------------------------------------------------------
// Span collection
// ---------------------------------------------------------------------------

fn collect_mdx_spans(node: &markdown::mdast::Node, spans: &mut Vec<MdxSpan>) {
  match node {
    markdown::mdast::Node::MdxjsEsm(_)
    | markdown::mdast::Node::MdxFlowExpression(_)
    | markdown::mdast::Node::MdxJsxFlowElement(_) => {
      if let Some(pos) = node.position() {
        spans.push(MdxSpan {
          start: pos.start.offset,
          end: pos.end.offset,
          formattable: true,
        });
      }
      return;
    }

    markdown::mdast::Node::Paragraph(_)
    | markdown::mdast::Node::Heading(_)
    | markdown::mdast::Node::TableCell(_)
      if has_inline_mdx_deep(node) =>
    {
      if let Some(pos) = node.position() {
        spans.push(MdxSpan {
          start: pos.start.offset,
          end: pos.end.offset,
          formattable: false,
        });
      }
      return;
    }

    _ => {}
  }
  if let Some(children) = node.children() {
    for child in children {
      collect_mdx_spans(child, spans);
    }
  }
}

fn has_inline_mdx_deep(node: &markdown::mdast::Node) -> bool {
  matches!(
    node,
    markdown::mdast::Node::MdxTextExpression(_) | markdown::mdast::Node::MdxJsxTextElement(_)
  ) || node
    .children()
    .is_some_and(|c| c.iter().any(has_inline_mdx_deep))
}

fn merge_spans(spans: Vec<MdxSpan>) -> Vec<MdxSpan> {
  let mut merged: Vec<MdxSpan> = Vec::new();
  for span in spans {
    if let Some(last) = merged.last_mut() {
      if span.start <= last.end {
        last.end = last.end.max(span.end);
        last.formattable = last.formattable && span.formattable;
        continue;
      }
    }
    merged.push(span);
  }
  merged
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn try_format_region(
  text: &str,
  line_width: u32,
  cb: &mut impl for<'a> FnMut(&str, &'a str, u32) -> Result<Option<String>, FormatError>,
) -> String {
  match cb("tsx", text, line_width) {
    Ok(Some(f)) => {
      let f = f.strip_suffix('\n').unwrap_or(&f);
      f.strip_suffix('\r').unwrap_or(f).to_string()
    }
    _ => text.to_string(),
  }
}

fn as_ignore_directive<'a>(text: &'a str, config: &Configuration) -> Option<&'a str> {
  let inner = text.strip_prefix('{')?.strip_suffix('}')?;
  let comment = inner.trim().strip_prefix("/*")?.strip_suffix("*/")?;
  let d = comment.trim();
  let directives = [
    config.ignore_directive.as_str(),
    config.ignore_start_directive.as_str(),
    config.ignore_end_directive.as_str(),
    config.ignore_file_directive.as_str(),
  ];
  directives.contains(&d).then_some(d)
}

fn find_nonce(source: &str) -> String {
  let mut n = String::from("__dprint_mdx_a");
  while source.contains(&n) {
    n.push('a');
  }
  n
}

fn normalize_newlines(text: &str, new_line: &str) -> String {
  if new_line == "\n" {
    text.replace("\r\n", "\n")
  } else {
    text.replace("\r\n", "\n").replace('\n', new_line)
  }
}

// ---------------------------------------------------------------------------
// Restoration
// ---------------------------------------------------------------------------

/// Restores placeholders in a single forward scan. Regions are consumed in
/// order, so even if two placeholders have the same text (e.g. two ignore
/// directives), each match restores the correct region.
///
/// Placeholders are only matched at the start of a line to avoid false
/// positives inside code spans or other inline content.
fn restore_regions(text: &str, regions: &[MdxRegion]) -> String {
  let mut result = String::with_capacity(text.len());
  let mut pos = 0;
  let mut next_region = 0;
  let bytes = text.as_bytes();

  while pos < text.len() {
    let at_line_start = pos == 0 || bytes[pos - 1] == b'\n';

    if at_line_start {
      // Try the next unconsumed region first.
      if next_region < regions.len() && text[pos..].starts_with(&regions[next_region].placeholder) {
        result.push_str(&regions[next_region].restored);
        pos += regions[next_region].placeholder.len();
        next_region += 1;
        continue;
      }
      // Check remaining regions (in case of reordering).
      let mut matched = false;
      let start = (next_region + 1).min(regions.len());
      for region in &regions[start..] {
        if text[pos..].starts_with(&region.placeholder) {
          result.push_str(&region.restored);
          pos += region.placeholder.len();
          matched = true;
          break;
        }
      }
      if matched {
        continue;
      }
    }

    let ch = text[pos..].chars().next().unwrap();
    result.push(ch);
    pos += ch.len_utf8();
  }
  result
}

//! MDX support via pre/post-processing.
//!
//! MDX files are markdown with embedded JSX, import/export statements, and
//! `{expression}` blocks. Rather than teaching the markdown parser about all
//! of these, we use `markdown-rs` (with `ParseOptions::mdx()`) to locate the
//! byte spans of every MDX-specific node, replace each one with a placeholder
//! that the markdown formatter will leave alone, format the resulting markdown,
//! and then put the original MDX text back in place of each placeholder.
//!
//! This approach is safe because `markdown-rs` is the reference Rust MDX
//! parser by the same author as the MDX specification.

use crate::configuration::Configuration;
use crate::format_text::FormatError;

/// A region of the source that holds MDX-specific syntax, together with the
/// text to substitute for it while the markdown formatter runs.
struct MdxRegion {
  start: usize,
  end: usize,
  /// What the formatter sees in place of the original text.
  placeholder: String,
  /// The original text, restored after formatting.
  original: String,
}

/// Formats an MDX file.
///
/// MDX-specific constructs (imports, exports, JSX components, expressions)
/// are protected from the markdown formatter and restored afterward.
/// The `format_code_block_text` callback is forwarded to the inner markdown
/// formatter for fenced code blocks.
pub fn format_mdx_text(
  file_text: &str,
  config: &Configuration,
  format_code_block_text: impl for<'a> FnMut(&str, &'a str, u32) -> Result<Option<String>, FormatError>,
) -> Result<Option<String>, FormatError> {
  // 1. Parse with markdown-rs to find MDX spans.
  let tree = match parse_mdx_tree(file_text) {
    Some(tree) => tree,
    // If markdown-rs can't parse it (invalid MDX), return the file unchanged
    // rather than risking corruption.
    None => return Ok(None),
  };

  let mut spans = Vec::new();
  collect_mdx_spans(&tree, &mut spans);
  spans.sort_by_key(|s| s.start);
  let spans = merge_spans(spans);

  // No MDX nodes at all — format as plain markdown.
  if spans.is_empty() {
    return crate::format_text(file_text, config, format_code_block_text);
  }

  // 2. Build regions with unique placeholders.
  let nonce = find_nonce(file_text);
  let regions = build_regions(file_text, &spans, &nonce, config);

  // 3. Build the placeholder'd text.
  let placeholdered = build_placeholdered_text(file_text, &regions);

  // 4. Format the placeholder'd text as markdown.
  let formatted = crate::format_text(&placeholdered, config, format_code_block_text)?;
  let formatted_text = formatted.as_deref().unwrap_or(&placeholdered);

  // 5. Restore originals in a single pass.
  let result = restore_regions(formatted_text, &regions);

  if result == file_text {
    Ok(None)
  } else {
    Ok(Some(result))
  }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses MDX source into an mdast tree.
fn parse_mdx_tree(source: &str) -> Option<markdown::mdast::Node> {
  let opts = markdown::ParseOptions {
    constructs: markdown::Constructs {
      // P1-1: Enable frontmatter so that YAML/TOML metadata blocks don't
      // confuse the MDX parser (e.g., `<Component>` inside `---` fences
      // would otherwise panic).
      frontmatter: true,
      ..markdown::Constructs::mdx()
    },
    // P1-2: Use Eof for incomplete expressions/ESM so that multi-line
    // constructs with blank lines aren't prematurely terminated.
    mdx_esm_parse: Some(Box::new(esm_parse)),
    mdx_expression_parse: Some(Box::new(expression_parse)),
    ..markdown::ParseOptions::mdx()
  };
  // Catch any remaining panics (markdown-rs is complex) rather than
  // aborting the process.
  std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| markdown::to_mdast(source, &opts)))
    .ok()?
    .ok()
}

/// ESM parse callback: returns `Ok` when braces are balanced, `Eof` when the
/// input looks incomplete (more `{` than `}`).
///
/// This is not a real JS parser, but it's enough to prevent markdown-rs from
/// splitting a multi-line `export function` at the first blank line. A real
/// JS parser (e.g., SWC) would be needed for full correctness with regex
/// literals like `/}/`.
fn esm_parse(value: &str) -> markdown::MdxSignal {
  if braces_balanced(value) {
    markdown::MdxSignal::Ok
  } else {
    markdown::MdxSignal::Eof(
      "Incomplete ESM".into(),
      Box::new("esm".into()),
      Box::new("esm".into()),
    )
  }
}

/// Expression parse callback: same brace-balancing logic.
fn expression_parse(value: &str, _kind: &markdown::MdxExpressionKind) -> markdown::MdxSignal {
  if braces_balanced(value) {
    markdown::MdxSignal::Ok
  } else {
    markdown::MdxSignal::Eof(
      "Incomplete expression".into(),
      Box::new("expression".into()),
      Box::new("expression".into()),
    )
  }
}

/// Whether the braces in the value are balanced, respecting string literals
/// and template literals (but not regex — that would require a full parser).
fn braces_balanced(value: &str) -> bool {
  let mut depth: i32 = 0;
  let mut in_single = false;
  let mut in_double = false;
  let mut in_template = false;
  let mut escaped = false;

  for ch in value.chars() {
    if escaped {
      escaped = false;
      continue;
    }
    if ch == '\\' && (in_single || in_double || in_template) {
      escaped = true;
      continue;
    }
    match ch {
      '\'' if !in_double && !in_template => in_single = !in_single,
      '"' if !in_single && !in_template => in_double = !in_double,
      '`' if !in_single && !in_double => in_template = !in_template,
      '{' if !in_single && !in_double && !in_template => depth += 1,
      '}' if !in_single && !in_double && !in_template => depth -= 1,
      _ => {}
    }
  }
  depth <= 0
}

// ---------------------------------------------------------------------------
// Span collection
// ---------------------------------------------------------------------------

/// Collects the byte spans of all MDX-specific nodes that need protection.
fn collect_mdx_spans(node: &markdown::mdast::Node, spans: &mut Vec<MdxSpan>) {
  match node {
    // Block-level MDX: ESM, flow expressions, flow JSX elements.
    markdown::mdast::Node::MdxjsEsm(_)
    | markdown::mdast::Node::MdxFlowExpression(_)
    | markdown::mdast::Node::MdxJsxFlowElement(_) => {
      if let Some(pos) = node.position() {
        spans.push(MdxSpan {
          start: pos.start.offset,
          end: pos.end.offset,
        });
      }
      return; // Don't recurse; the whole subtree is MDX.
    }

    // P1-5: Any phrasing container that holds inline MDX must be protected
    // as a whole unit. This covers Paragraph, Heading, and (with GFM)
    // TableCell — all containers whose text content the markdown formatter
    // would otherwise reflow or normalise.
    markdown::mdast::Node::Paragraph(_) | markdown::mdast::Node::Heading(_) | markdown::mdast::Node::TableCell(_)
      if has_inline_mdx_deep(node) =>
    {
      if let Some(pos) = node.position() {
        spans.push(MdxSpan {
          start: pos.start.offset,
          end: pos.end.offset,
        });
      }
      return;
    }

    _ => {}
  }

  // Recurse for containers (root, blockquote, list items, etc.)
  if let Some(children) = node.children() {
    for child in children {
      collect_mdx_spans(child, spans);
    }
  }
}

struct MdxSpan {
  start: usize,
  end: usize,
}

/// Whether a node or any of its descendants is an inline MDX node.
fn has_inline_mdx_deep(node: &markdown::mdast::Node) -> bool {
  if matches!(
    node,
    markdown::mdast::Node::MdxTextExpression(_) | markdown::mdast::Node::MdxJsxTextElement(_)
  ) {
    return true;
  }
  node
    .children()
    .map(|children| children.iter().any(has_inline_mdx_deep))
    .unwrap_or(false)
}

/// Merges overlapping or adjacent spans.
fn merge_spans(spans: Vec<MdxSpan>) -> Vec<MdxSpan> {
  let mut merged: Vec<MdxSpan> = Vec::new();
  for span in spans {
    if let Some(last) = merged.last_mut() {
      if span.start <= last.end {
        last.end = last.end.max(span.end);
        continue;
      }
    }
    merged.push(span);
  }
  merged
}

// ---------------------------------------------------------------------------
// Placeholder management
// ---------------------------------------------------------------------------

/// P1-4: Finds a nonce string that does not appear anywhere in the source,
/// so placeholders built from it can never collide with user content.
fn find_nonce(source: &str) -> String {
  let mut nonce = String::from("__dprint_mdx_a");
  while source.contains(&nonce) {
    nonce.push('a');
  }
  nonce
}

/// Builds the list of regions, each with its unique placeholder and the
/// original text to restore.
///
/// P1-3: Ignore directives (`{/* dprint-ignore */}` etc.) are translated
/// to HTML comments for the formatter, but the *original* MDX form is what
/// gets restored — so the output never contains invalid HTML comments.
fn build_regions(source: &str, spans: &[MdxSpan], nonce: &str, config: &Configuration) -> Vec<MdxRegion> {
  spans
    .iter()
    .enumerate()
    .map(|(i, span)| {
      let original = source[span.start..span.end].to_string();
      let placeholder = if let Some(html) = as_ignore_html_comment(&original, config) {
        html
      } else {
        format!("<!-- {} {} -->", nonce, i)
      };
      MdxRegion {
        start: span.start,
        end: span.end,
        placeholder,
        original,
      }
    })
    .collect()
}

/// Builds the text with all MDX regions replaced by their placeholders.
fn build_placeholdered_text(source: &str, regions: &[MdxRegion]) -> String {
  let mut result = String::with_capacity(source.len());
  let mut pos = 0;
  for region in regions {
    result.push_str(&source[pos..region.start]);
    result.push_str(&region.placeholder);
    pos = region.end;
  }
  result.push_str(&source[pos..]);
  result
}

/// If the MDX text is a `{/* <directive> */}` expression, returns the
/// equivalent `<!-- <directive> -->` HTML comment.
fn as_ignore_html_comment(text: &str, config: &Configuration) -> Option<String> {
  let inner = text.strip_prefix('{')?.strip_suffix('}')?;
  let inner = inner.trim();
  let comment = inner.strip_prefix("/*")?.strip_suffix("*/")?;
  let directive = comment.trim();

  let directives = [
    &config.ignore_directive,
    &config.ignore_start_directive,
    &config.ignore_end_directive,
    &config.ignore_file_directive,
  ];
  if directives.iter().any(|d| d.as_str() == directive) {
    Some(format!("<!-- {} -->", directive))
  } else {
    None
  }
}

/// P1-4: Restores all placeholders in a single scan, avoiding repeated
/// global replacements that could cascade or collide.
fn restore_regions(text: &str, regions: &[MdxRegion]) -> String {
  let mut result = String::with_capacity(text.len());
  let mut search_from = 0;

  // For each position in the formatted text, check if any placeholder starts
  // here. Because placeholders are unique (nonce-based) and non-overlapping,
  // a simple linear scan works.
  'outer: while search_from < text.len() {
    for region in regions {
      if text[search_from..].starts_with(&region.placeholder) {
        result.push_str(&region.original);
        search_from += region.placeholder.len();
        continue 'outer;
      }
    }
    // Copy one character (respecting UTF-8 boundaries).
    let ch = text[search_from..].chars().next().unwrap();
    result.push(ch);
    search_from += ch.len_utf8();
  }

  result
}

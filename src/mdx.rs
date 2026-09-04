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

/// The prefix every placeholder starts with. Chosen so that the markdown
/// parser reads each one as an HTML comment block (which it preserves).
const PLACEHOLDER_PREFIX: &str = "<!-- __dprint_mdx_";
const PLACEHOLDER_SUFFIX: &str = " -->";

/// A region of the source that holds MDX-specific syntax.
struct MdxSpan {
  start: usize,
  end: usize,
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

  // 2. Replace MDX spans with placeholders.
  let originals: Vec<&str> = spans.iter().map(|s| &file_text[s.start..s.end]).collect();
  let placeholdered = replace_spans(file_text, &spans, &originals, config);

  // 3. Format the placeholder'd text as markdown.
  let formatted = crate::format_text(&placeholdered, config, format_code_block_text)?;
  let formatted = formatted.as_deref().unwrap_or(&placeholdered);

  // 4. Restore originals.
  let result = restore_placeholders(formatted, &originals);

  if result == file_text {
    Ok(None)
  } else {
    Ok(Some(result))
  }
}

/// Parses MDX source into an mdast tree.
fn parse_mdx_tree(source: &str) -> Option<markdown::mdast::Node> {
  let opts = markdown::ParseOptions {
    // Accept any ESM / expression content without requiring a JS parser.
    mdx_esm_parse: Some(Box::new(|_| markdown::MdxSignal::Ok)),
    mdx_expression_parse: Some(Box::new(|_, _| markdown::MdxSignal::Ok)),
    ..markdown::ParseOptions::mdx()
  };
  markdown::to_mdast(source, &opts).ok()
}

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

    // A paragraph that contains inline MDX (expressions or JSX text elements)
    // must be protected as a whole, because substituting inline placeholders
    // can change how the markdown formatter treats the paragraph (e.g., adding
    // blank lines, mangling whitespace in JS strings).
    markdown::mdast::Node::Paragraph(_) => {
      if let Some(children) = node.children() {
        let has_inline_mdx = children.iter().any(|child| {
          matches!(
            child,
            markdown::mdast::Node::MdxTextExpression(_) | markdown::mdast::Node::MdxJsxTextElement(_)
          ) || has_inline_mdx_deep(child)
        });
        if has_inline_mdx {
          if let Some(pos) = node.position() {
            spans.push(MdxSpan {
              start: pos.start.offset,
              end: pos.end.offset,
            });
          }
          return; // Protect the entire paragraph.
        }
      }
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

/// Replaces each MDX span with a placeholder.
///
/// Block-level spans become HTML comment placeholders. MDX flow expressions
/// that are `{/* dprint-ignore */}` (or variants) become the equivalent HTML
/// comment so the formatter's ignore machinery recognises them.
fn replace_spans(source: &str, spans: &[MdxSpan], originals: &[&str], config: &Configuration) -> String {
  let mut result = String::with_capacity(source.len());
  let mut pos = 0;
  for (i, span) in spans.iter().enumerate() {
    result.push_str(&source[pos..span.start]);

    // Check if this is a dprint-ignore expression that should be translated
    // to an HTML comment rather than a generic placeholder.
    if let Some(html) = as_ignore_html_comment(originals[i], config) {
      result.push_str(&html);
    } else {
      result.push_str(PLACEHOLDER_PREFIX);
      result.push_str(&i.to_string());
      result.push_str(PLACEHOLDER_SUFFIX);
    }
    pos = span.end;
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

/// Restores each placeholder with its original text.
fn restore_placeholders(text: &str, originals: &[&str]) -> String {
  let mut result = text.to_string();
  for (i, original) in originals.iter().enumerate() {
    let placeholder = format!("{}{}{}", PLACEHOLDER_PREFIX, i, PLACEHOLDER_SUFFIX);
    result = result.replace(&placeholder, original);
  }
  result
}

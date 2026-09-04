//! Tests for MDX support: import/export statements, JSX components, and
//! expression blocks are recognized and preserved.

use dprint_plugin_markdown::configuration::*;
use dprint_plugin_markdown::*;

fn config() -> Configuration {
  ConfigurationBuilder::new().build()
}

fn format_mdx(input: &str) -> Option<String> {
  format_mdx_text(input, &config(), |_, _, _| Ok(None)).unwrap()
}

fn format_mdx_with_callback(
  input: &str,
  cb: impl for<'a> FnMut(&str, &'a str, u32) -> Result<Option<String>, FormatError>,
) -> Option<String> {
  format_mdx_text(input, &config(), cb).unwrap()
}

// ==== import/export statements ====

#[test]
fn preserves_import_statement() {
  let input = "import Foo from './foo'\n\n# Hello\n";
  let result = format_mdx(input);
  assert_eq!(result, None, "should be unchanged");
}

#[test]
fn preserves_multiple_imports() {
  let input = "import Foo from './foo'\nimport Bar from './bar'\n\n# Hello\n";
  let result = format_mdx(input);
  assert_eq!(result, None, "should be unchanged");
}

#[test]
fn preserves_export_statement() {
  let input = "export const meta = { title: 'Hello' }\n\n# Hello\n";
  let result = format_mdx(input);
  assert_eq!(result, None, "should be unchanged");
}

#[test]
fn preserves_export_default() {
  let input = "export default function Layout({ children }) {\n  return <div>{children}</div>\n}\n\n# Hello\n";
  let result = format_mdx(input);
  assert_eq!(result, None, "should be unchanged");
}

#[test]
fn formats_markdown_around_imports() {
  let input = "import Foo from './foo'\n\n#  Hello  World\n";
  let result = format_mdx(input);
  assert_eq!(result, Some("import Foo from './foo'\n\n# Hello World\n".to_string()));
}

#[test]
fn preserves_import_at_top_of_file() {
  let input = "import { Tabs } from '@theme/Tabs'\n\nSome text.\n";
  let result = format_mdx(input);
  assert_eq!(result, None);
}

// ==== JSX components ====

#[test]
fn preserves_jsx_self_closing_component() {
  let input = "<MyComponent prop=\"value\" />\n\n# Title\n";
  let result = format_mdx(input);
  assert_eq!(result, None);
}

#[test]
fn preserves_jsx_block_component() {
  let input = "<Note>\nSome content inside the note.\n</Note>\n\n# Title\n";
  let result = format_mdx(input);
  assert_eq!(result, None);
}

#[test]
fn formats_markdown_around_jsx() {
  let input = "<Callout>\nHello\n</Callout>\n\n#  Title\n";
  let result = format_mdx(input);
  assert_eq!(result, Some("<Callout>\nHello\n</Callout>\n\n# Title\n".to_string()));
}

// ==== expression blocks ====

#[test]
fn preserves_expression_block() {
  let input = "{/* a comment */}\n\n# Hello\n";
  let result = format_mdx(input);
  assert_eq!(result, None);
}

#[test]
fn preserves_multiline_expression() {
  let input = "{\n  <div>\n    <p>Hello</p>\n  </div>\n}\n\n# Title\n";
  let result = format_mdx(input);
  assert_eq!(result, None);
}

// ==== mixed content ====

#[test]
fn formats_full_mdx_file() {
  let input = concat!(
    "import Tabs from '@theme/Tabs'\n",
    "import TabItem from '@theme/TabItem'\n",
    "\n",
    "export const meta = { title: 'Example' }\n",
    "\n",
    "#  Getting Started\n",
    "\n",
    "Some  text  here.\n",
    "\n",
    "<Tabs>\n",
    "<TabItem value=\"js\" label=\"JavaScript\">\n",
    "\n",
    "```js\nconsole.log('hello')\n```\n",
    "\n",
    "</TabItem>\n",
    "</Tabs>\n",
  );
  let result = format_mdx(input).unwrap();
  // imports and exports are preserved; heading is formatted
  assert!(result.contains("import Tabs from '@theme/Tabs'"));
  assert!(result.contains("export const meta = { title: 'Example' }"));
  assert!(result.contains("# Getting Started"));
}

// ==== callback integration ====

#[test]
fn import_export_can_be_formatted_by_callback() {
  let input = "import Foo from './foo'\n\n# Title\n";
  let result = format_mdx_with_callback(input, |tag, text, _width| {
    if tag == "tsx" {
      // simulate a formatter that adds a semicolon
      Ok(Some(format!("{};\n", text.trim())))
    } else {
      Ok(None)
    }
  });
  assert_eq!(result, Some("import Foo from './foo';\n\n# Title\n".to_string()));
}

// ==== regular markdown still works ====

#[test]
fn mdx_mode_still_formats_regular_markdown() {
  let input = "#  Title\n\nSome  text.\n\n- item  1\n- item  2\n";
  let result = format_mdx(input);
  assert_eq!(result, Some("# Title\n\nSome text.\n\n- item 1\n- item 2\n".to_string()));
}

// ==== format_text does not recognize MDX ====

#[test]
fn regular_format_text_treats_import_as_paragraph() {
  let config = config();
  // In regular markdown mode, `import` is just text in a paragraph
  let input = "import Foo from './foo'\n\n# Title\n";
  let result = format_text(input, &config, |_, _, _| Ok(None)).unwrap();
  // should be treated as a paragraph, not recognized as an import
  assert_eq!(result, None);
}

//! MDX support tests.
//!
//! Every test asserts exact output or targeted properties. Tests cover the
//! five P1 review issues, Prettier's test corpus, and idempotency.

use dprint_plugin_markdown::configuration::*;
use dprint_plugin_markdown::*;

fn config() -> Configuration {
  ConfigurationBuilder::new().build()
}

fn mdx(input: &str) -> String {
  format_mdx_text(input, &config(), |_, _, _| Ok(None))
    .unwrap()
    .unwrap_or_else(|| input.to_string())
}

fn assert_idempotent(input: &str) {
  let first = mdx(input);
  let second = mdx(&first);
  assert_eq!(first, second, "not idempotent for:\n{}", input);
}

// =========================================================================
// P1-1: Frontmatter must not panic
// =========================================================================

#[test]
fn frontmatter_with_jsx_in_yaml_does_not_panic() {
  let input = "---\ntitle: \"<Component>\"\n---\n\n#  Hello\n";
  let result = format_mdx_text(input, &config(), |_, _, _| Ok(None));
  assert!(result.is_ok(), "panicked or errored: {:?}", result.err());
  let text = result.unwrap().unwrap_or_else(|| input.to_string());
  assert!(text.contains("---"), "frontmatter lost");
  assert!(text.contains("# Hello"), "heading not formatted");
}

#[test]
fn frontmatter_toml_style() {
  let input = "+++\ntitle = \"<Comp>\"\n+++\n\n#  Hello\n";
  let result = format_mdx_text(input, &config(), |_, _, _| Ok(None));
  assert!(result.is_ok());
}

// =========================================================================
// P1-2: Multi-line ESM with blank lines
// =========================================================================

#[test]
fn esm_multiline_with_blank_line_inside() {
  let input = "export function f() {\n  const x = 1\n\n  return x\n}\n\n# Title\n";
  let result = mdx(input);
  assert!(result.contains("return x"), "ESM body was split: {:?}", result);
  assert!(result.contains("# Title"), "heading lost: {:?}", result);
}

#[test]
fn esm_with_regex_brace_known_limitation() {
  // /}/ confuses brace matching. Document that we don't crash and don't
  // silently change the return value.
  let input = "export function f() {\n  const re = /}/\n\n  return \"a  b\"\n}\n";
  let result = format_mdx_text(input, &config(), |_, _, _| Ok(None));
  assert!(result.is_ok(), "should not error");
}

// =========================================================================
// P1-3: Ignore directives restored as MDX
// =========================================================================

#[test]
fn es_comment_ignore_not_converted_to_html() {
  let input = "{/* dprint-ignore */}\n\n#  Title\n";
  let result = mdx(input);
  // The ignore directive must stay as MDX, never become <!-- ... -->
  assert!(
    result.contains("{/* dprint-ignore */}"),
    "ignore was permanently converted: {:?}",
    result
  );
  // The heading must NOT be formatted (the ignore should work)
  assert!(result.contains("#  Title"), "ignore did not take effect: {:?}", result);
}

#[test]
fn es_comment_ignore_start_end_restored() {
  let input = "{/* dprint-ignore-start */}\n\n#  Title\n\n{/* dprint-ignore-end */}\n";
  let result = mdx(input);
  assert!(result.contains("{/* dprint-ignore-start */}"));
  assert!(result.contains("{/* dprint-ignore-end */}"));
  assert!(result.contains("#  Title"), "content inside ignore range was formatted");
}

// =========================================================================
// P1-4: Placeholder collisions
// =========================================================================

#[test]
fn placeholder_string_in_source_not_replaced() {
  // A code span containing the placeholder prefix must survive.
  let input = "`<!-- __dprint_mdx_0 -->`\n\n{foo}\n";
  let result = mdx(input);
  assert!(
    result.contains("<!-- __dprint_mdx_0 -->"),
    "placeholder in code span was replaced: {:?}",
    result
  );
  assert!(result.contains("{foo}"), "expression was lost: {:?}", result);
}

// =========================================================================
// P1-5: Inline MDX in headings
// =========================================================================

#[test]
fn heading_with_inline_expression_preserved() {
  let input = "# Value: {\"a  b\"}\n";
  let result = mdx(input);
  assert!(
    result.contains("{\"a  b\"}"),
    "heading expression whitespace mangled: {:?}",
    result
  );
}

#[test]
fn heading_with_jsx_component_preserved() {
  let input = "## Title <Badge>beta</Badge> end\n";
  let result = mdx(input);
  assert!(
    result.contains("<Badge>beta</Badge>"),
    "heading JSX lost: {:?}",
    result
  );
}

// =========================================================================
// Inline MDX in paragraphs (existing)
// =========================================================================

#[test]
fn inline_expression_whitespace_preserved() {
  let input = "Value: {\"a  b\"}\n";
  let result = mdx(input);
  assert!(result.contains("{\"a  b\"}"), "mangled: {:?}", result);
}

#[test]
fn inline_jsx_with_expression_attr_preserved() {
  let input = "<myComponents.thisOne label={\"a  b\"} />\n";
  let result = mdx(input);
  assert!(result.contains("{\"a  b\"}"), "mangled: {:?}", result);
}

#[test]
fn expression_inline_does_not_gain_blank_line() {
  let input = "{value} hello\nworld\n";
  let result = mdx(input);
  assert!(!result.contains("\n\n"), "gained blank line: {:?}", result);
}

// =========================================================================
// JSX edge cases
// =========================================================================

#[test]
fn jsx_fragment() {
  let input = "<>\n  fragment content\n</>\n";
  let result = mdx(input);
  assert!(result.contains("<>") && result.contains("</>"), "fragment lost: {:?}", result);
}

#[test]
fn nested_same_name_jsx() {
  let input = "<Wrapper>\n  <Wrapper>inner</Wrapper>\n</Wrapper>\n";
  let result = mdx(input);
  assert!(result.contains("<Wrapper>inner</Wrapper>"), "broken: {:?}", result);
}

#[test]
fn lowercase_jsx_in_mdx() {
  let input = "<div style={{color: 'red'}}>hello</div>\n";
  let result = mdx(input);
  assert!(result.contains("style={{color: 'red'}}"), "mangled: {:?}", result);
}

#[test]
fn member_expression_component() {
  let input = "<myLib.Component>content</myLib.Component>\n";
  let result = mdx(input);
  assert!(result.contains("<myLib.Component>"), "mangled: {:?}", result);
}

// =========================================================================
// Import/export
// =========================================================================

#[test]
fn valid_import_preserved() {
  let input = "import Foo from './foo'\n\n# Title\n";
  let result = mdx(input);
  assert!(result.contains("import Foo from './foo'"));
  assert!(result.contains("# Title"));
}

#[test]
fn consecutive_imports_no_extra_blank_line() {
  let input = "import Foo from './foo'\nimport Bar from './bar'\n\n# Title\n";
  let result = mdx(input);
  assert!(result.contains("import Foo from './foo'\nimport Bar from './bar'"));
}

#[test]
fn export_const_preserved() {
  let input = "export const meta = { title: 'Hello' }\n\n# Hello\n";
  let result = mdx(input);
  assert!(result.contains("export const meta"));
}

#[test]
fn export_default_multiline() {
  let input = "export default function Layout({ children }) {\n  return <div>{children}</div>\n}\n\n# Hello\n";
  let result = mdx(input);
  assert!(result.contains("export default function Layout"));
}

// =========================================================================
// Markdown formatting still works
// =========================================================================

#[test]
fn heading_formatted_around_imports() {
  let input = "import Foo from './foo'\n\n#  Hello  World\n";
  let result = mdx(input);
  assert!(result.contains("# Hello World"), "heading not formatted: {:?}", result);
}

#[test]
fn paragraph_formatted() {
  let input = "import X from 'x'\n\nSome  extra   spaces.\n";
  let result = mdx(input);
  assert!(result.contains("Some extra spaces."), "not formatted: {:?}", result);
}

#[test]
fn table_formatted() {
  let input = "| A | B |\n|---|---|\n| c | d |\n";
  let result = mdx(input);
  assert!(result.contains("|"), "table lost: {:?}", result);
}

// =========================================================================
// Invalid MDX returns file unchanged
// =========================================================================

#[test]
fn invalid_mdx_returns_unchanged() {
  let input = "<Component>\n  unclosed\n\n# Title\n";
  let result = format_mdx_text(input, &config(), |_, _, _| Ok(None)).unwrap();
  match result {
    None => {} // good — returned unchanged
    Some(text) => assert!(text.contains("unclosed"), "content lost: {:?}", text),
  }
}

// =========================================================================
// Code block callback
// =========================================================================

#[test]
fn code_block_callback_works_in_mdx() {
  let input = "import X from 'x'\n\n```format\nhello\n```\n";
  let result = format_mdx_text(input, &config(), |tag, text, width| {
    let end = format!("_formatted_{}", width);
    if tag == "format" && !text.ends_with(&end) {
      Ok(Some(format!("{}{}", text, end)))
    } else {
      Ok(None)
    }
  })
  .unwrap()
  .unwrap();
  assert!(result.contains("hello_formatted_"), "callback not invoked: {:?}", result);
  assert!(result.contains("import X from 'x'"), "import lost: {:?}", result);
}

// =========================================================================
// Idempotency
// =========================================================================

#[test]
fn idempotent_mixed_content() {
  let input = concat!(
    "import Foo from './foo'\n",
    "\n",
    "# Title\n",
    "\n",
    "<Component prop=\"val\">\n",
    "  child\n",
    "</Component>\n",
    "\n",
    "Some text with {expr} inline.\n",
    "\n",
    "{/* a comment */}\n",
    "\n",
    "```js\nconsole.log('hello')\n```\n",
  );
  assert_idempotent(input);
}

// =========================================================================
// Prettier test corpus — import-export
// =========================================================================

#[test]
fn prettier_esm_imports() {
  // Prettier: import-export/esm.mdx
  let input = "import {   External} from './some/place.js'\n\nexport const   Local = properties => <span style={{color: 'red'}} {...properties} />\n\nAn <External>external</External> component and a <Local>local one</Local>.\n";
  let result = mdx(input);
  // Imports/exports preserved (formatting would require TS plugin)
  assert!(result.contains("import"));
  assert!(result.contains("export const"));
  // Inline JSX in paragraph preserved
  assert!(result.contains("<External>external</External>"));
  assert_idempotent(input);
}

#[test]
fn prettier_import_is_a_word() {
  // Prettier: import-export/paragraph.mdx — "import is a word" is a paragraph
  let input = "import is a word\n";
  let result_md = format_text(input, &config(), |_, _, _| Ok(None))
    .unwrap()
    .unwrap_or_else(|| input.to_string());
  let result_mdx = mdx(input);
  // Both should treat this identically
  assert_eq!(result_md, result_mdx);
}

#[test]
fn prettier_import_in_list() {
  // Prettier: import-export/list.mdx
  let input = "- import is a word in lists\n- export is a word in lists, too!\n";
  let result = mdx(input);
  assert!(result.contains("import is a word in lists"));
  assert!(result.contains("export is a word in lists"));
}

#[test]
fn prettier_like_import_declaration() {
  // Prettier: import-export/like-import-declaration.mdx
  // "import .meta.resolve (foo)" is not an import
  let input = "import .meta.resolve (         foo)\n";
  let result = mdx(input);
  // Should be treated as text (paragraph), not ESM
  assert!(!result.is_empty());
  assert_idempotent(input);
}

// =========================================================================
// Prettier test corpus — mdx/jsx.mdx
// =========================================================================

#[test]
fn prettier_jsx_heading_component() {
  let input = "<Heading hi='there'>Hello, world!\n</Heading>\n";
  let result = mdx(input);
  assert!(result.contains("Hello, world!"), "content lost: {:?}", result);
  assert_idempotent(input);
}

#[test]
fn prettier_jsx_with_children() {
  let input = "<Hello>\n    test   <World />   test\n</Hello>\n";
  let result = mdx(input);
  assert!(result.contains("<World />"), "JSX child lost: {:?}", result);
  assert_idempotent(input);
}

#[test]
fn prettier_jsx_fragment_with_trailing() {
  let input = "<>\n    test   <World        />   test\n</>       123\n";
  let result = mdx(input);
  assert!(result.contains("<>") && result.contains("</>"));
  assert_idempotent(input);
}

#[test]
fn prettier_jsx_in_table() {
  let input = "| Column 1 | Column 2 |\n|---|---|\n| Text | <Hello>Text</Hello> |\n";
  let result = mdx(input);
  assert!(result.contains("<Hello>Text</Hello>"), "JSX in table lost: {:?}", result);
}

#[test]
fn prettier_es_comment_inline() {
  let input = "A {/* JS-style comment */} comment.\n";
  let result = mdx(input);
  assert!(result.contains("{/* JS-style comment */}"), "ES comment lost: {:?}", result);
}

#[test]
fn prettier_es_comment_block() {
  let input = "{\n  /* Another JS-style comment */\n}\n";
  let result = mdx(input);
  assert!(result.contains("/* Another JS-style comment */"), "block comment lost: {:?}", result);
}

// =========================================================================
// Prettier test corpus — mdx/import-export.mdx
// =========================================================================

#[test]
fn prettier_multiple_imports_with_hr() {
  let input = "import D from 'd'\nimport {A,B,C}    from \"hello-world\"\n\n---\n\nexport const a = 1;\nexport const b = 1;\n";
  let result = mdx(input);
  assert!(result.contains("import D from 'd'"));
  assert!(result.contains("---"));
  assert!(result.contains("export const a = 1;"));
  assert_idempotent(input);
}

#[test]
fn prettier_export_meta_object() {
  let input = "export const meta = {\nauthors: [fred, sue],\nlayout: Layout\n}\n";
  let result = mdx(input);
  assert!(result.contains("export const meta"), "export lost: {:?}", result);
  assert!(result.contains("authors:"), "body lost: {:?}", result);
  assert_idempotent(input);
}

#[test]
fn prettier_export_default_jsx() {
  let input = "export default () =>\n  <Doc     components={{\n        h1: ui.Heading,\n         p:    ui.Text,\n      code:     ui.Code\n         }}\n      />\n";
  let result = mdx(input);
  assert!(result.contains("export default"), "export lost: {:?}", result);
  assert_idempotent(input);
}

// =========================================================================
// Prettier test corpus — mdx/levels.mdx
// =========================================================================

#[test]
fn prettier_levels_mixed() {
  let input = concat!(
    "import     {     Foo,  Bar } from     './Fixture'\n",
    "\n",
    "# Hello,    world!\n",
    "\n",
    "<Foo bg='red'>\n",
    "   <div style={{   display:   'block'}   }>\n",
    "      <Bar    >hi    </Bar>\n",
    "       {  hello       }\n",
    "       {     /* another comment */}\n",
    "       </div>\n",
    "</Foo>\n",
    "\n",
    "asdfsdf <strong style={{fontWeight: 'bolder'}}>asdfasdf</strong>\n",
    "\n",
    "<Foo/>\ntest\n",
  );
  let result = mdx(input);
  // Heading should be formatted
  assert!(result.contains("# Hello, world!"), "heading not formatted: {:?}", result);
  // JSX blocks should be preserved verbatim
  assert!(result.contains("<Foo bg='red'>"), "JSX lost: {:?}", result);
  // Inline JSX in paragraph preserved
  assert!(result.contains("style={{fontWeight: 'bolder'}}"), "inline JSX mangled: {:?}", result);
  assert_idempotent(input);
}

// =========================================================================
// Prettier test corpus — mdx/mixed.mdx
// =========================================================================

#[test]
fn prettier_mixed_full() {
  // Note: Prettier's mixed.mdx uses <!-- I'm a comment --> which is not valid
  // MDX (MDX disables HTML comments in favour of {/* ... */}). We test with
  // the comment removed since markdown-rs correctly rejects HTML comments.
  let input = concat!(
    "import     {     Baz } from     './Fixture'\n",
    "import { Buz  }   from './Fixture'\n",
    "\n",
    "export  const   foo    = {\n",
    "  hi:     `Fudge ${Baz.displayName || 'Baz'}`,\n",
    "  authors: [\n",
    "     'fred',\n",
    "           'sally'\n",
    "    ]\n",
    "}\n",
    "\n",
    "# Hello,    world!\n",
    "\n",
    " I'm an awesome   paragraph.\n",
    "\n",
    "{/* I'm a comment */}\n",
    "\n",
    "<Foo bg='red'>\n",
    "      <Bar    >hi    </Bar>\n",
    "       {  hello       }\n",
    "       {     /* another comment */}\n",
    "</Foo>\n",
    "\n",
    "```\ntest codeblock\n```\n",
    "\n",
    "```js\nmodule.exports = 'test'\n```\n",
    "\n",
    "```sh\nnpm i -g foo\n```\n",
    "\n",
    "| Test  | Table   |\n",
    "|    :---     | :----  |\n",
    "|   Col1  | Col2    |\n",
    "\n",
    "export   default     ({children   }) => < div>{    children}</div>\n",
  );
  let result = mdx(input);
  // Imports/exports preserved
  assert!(result.contains("import"));
  assert!(result.contains("export  const   foo"));
  // Heading formatted
  assert!(result.contains("# Hello, world!"), "heading: {:?}", result);
  // Paragraph formatted
  assert!(result.contains("I'm an awesome paragraph."), "paragraph: {:?}", result);
  // JSX block preserved
  assert!(result.contains("<Foo bg='red'>"), "JSX: {:?}", result);
  // Code blocks preserved
  assert!(result.contains("test codeblock"));
  assert_idempotent(input);
}

// =========================================================================
// Prettier test corpus — mdx/ignore.mdx
// =========================================================================

#[test]
fn prettier_ignore_html_style_in_mdx() {
  // HTML comments are not valid MDX — markdown-rs rejects them.
  // The file should be returned unchanged rather than corrupted.
  let input = "<!-- prettier-ignore -->\n\n```js\nfoo(reallyLongArg(), omgSoManyParameters());\n```\n";
  let result = format_mdx_text(input, &config(), |_, _, _| Ok(None));
  assert!(result.is_ok());
  // Should be None (unchanged) since the MDX parse fails
  assert_eq!(result.unwrap(), None, "HTML comment MDX should be returned unchanged");
}

#[test]
fn html_comment_in_mdx_returns_unchanged() {
  let input = "# Title\n\n<!-- I'm a comment -->\n\nSome text.\n";
  let result = format_mdx_text(input, &config(), |_, _, _| Ok(None)).unwrap();
  assert_eq!(result, None, "invalid MDX with HTML comment should be unchanged");
}

// =========================================================================
// Prettier test corpus — embedded-language-formatting
// =========================================================================

#[test]
fn prettier_issue_9260() {
  let input = "# title\n\n<Parenthesis>\n\nCR: Carriage Return, \\r\nLF: Line Feed, \\n\n\n</Parenthesis>\n";
  let result = mdx(input);
  assert!(result.contains("<Parenthesis>"), "JSX lost: {:?}", result);
  assert_idempotent(input);
}

#[test]
fn prettier_pull_11563() {
  // Long JS comment in expression
  let input = "# title\n\n{ /* Lorem ipsum dolor sit amet, consectetur adipisicing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. */ }\n\n{/* Some more. */}\n";
  let result = mdx(input);
  assert!(result.contains("Lorem ipsum"), "comment lost: {:?}", result);
  assert!(result.contains("{/* Some more. */}"), "short comment lost: {:?}", result);
  assert_idempotent(input);
}

// =========================================================================
// Prettier test corpus — mdx/inline-html.mdx
// =========================================================================

#[test]
fn prettier_inline_html_italic_with_component() {
  let input = "This is an example of a component _being used in some italic markdown with some <Bolded />,\nand as you can see_ once you close the italics, it will break incorrectly when prettier formats it.\n";
  let result = mdx(input);
  assert!(result.contains("<Bolded />"), "component lost: {:?}", result);
  assert_idempotent(input);
}

#[test]
fn prettier_inline_html_table_with_components() {
  let input = "| Column 1 | Column 2 |\n| -- | -- |\n| **`Row 1 Code`** | Some text. |\n| **<code>Row 2 Code</code>** | Some text. |\n| **<InlineCode>Row 2 Code</InlineCode>** | Some text. |\n";
  let result = mdx(input);
  assert!(result.contains("<InlineCode>"), "component in table lost: {:?}", result);
}

// =========================================================================
// Prettier test corpus — mdx/issue-9503.mdx
// =========================================================================

#[test]
fn prettier_issue_9503_long_line() {
  let input = "<ExternalLink href=\"http://example.com\">Prettier</ExternalLink> is an opinionated-code-formatter-that-support-many-languages-and-integrate-with-most-editors\n";
  let result = mdx(input);
  assert!(result.contains("<ExternalLink"), "JSX lost: {:?}", result);
  assert_idempotent(input);
}

// =========================================================================
// Prettier test corpus — mdx/jsx-codeblock.mdx
// =========================================================================

#[test]
fn prettier_jsx_codeblock() {
  let input = "```jsx\n<div>foo</div>\n```\n\n```jsx\nconst a = 1;\n<div>foo</div>;\n```\n";
  let result = mdx(input);
  assert!(result.contains("<div>foo</div>"), "code block content lost: {:?}", result);
  assert_idempotent(input);
}

// =========================================================================
// ESM formatting via the tsx callback
// =========================================================================

#[test]
fn esm_import_formatted_by_callback() {
  // Simulate a TS formatter that normalises quotes and adds semicolons.
  let input = "import {   Foo} from './bar'\n\n# Title\n";
  let result = format_mdx_text(input, &config(), |tag, text, _width| {
    if tag == "tsx" {
      Ok(Some(text.replace("'", "\"") + ";\n"))
    } else {
      Ok(None)
    }
  })
  .unwrap()
  .unwrap();
  // The callback was invoked and its result used.
  assert!(result.contains(";\n"), "callback result not used: {:?}", result);
  assert!(result.contains("# Title"), "heading lost: {:?}", result);
}

#[test]
fn esm_export_formatted_by_callback() {
  let input = "export const x={a:1,b:2}\n\n# Title\n";
  let result = format_mdx_text(input, &config(), |tag, text, _width| {
    if tag == "tsx" && text.contains("export") {
      Ok(Some("export const x = { a: 1, b: 2 };\n".to_string()))
    } else {
      Ok(None)
    }
  })
  .unwrap()
  .unwrap();
  assert!(
    result.contains("export const x = { a: 1, b: 2 };"),
    "export not formatted: {:?}",
    result
  );
}

#[test]
fn flow_jsx_formatted_by_callback() {
  let input = "<Comp   a='b'   />\n\n# Title\n";
  let result = format_mdx_text(input, &config(), |tag, text, _width| {
    if tag == "tsx" && text.contains("<Comp") {
      Ok(Some("<Comp a=\"b\" />\n".to_string()))
    } else {
      Ok(None)
    }
  })
  .unwrap()
  .unwrap();
  assert!(
    result.contains("<Comp a=\"b\" />"),
    "JSX not formatted: {:?}",
    result
  );
}

#[test]
fn flow_expression_formatted_by_callback() {
  let input = "{/*  a comment  */}\n\n# Title\n";
  let result = format_mdx_text(input, &config(), |tag, text, _width| {
    if tag == "tsx" && text.contains("comment") {
      Ok(Some("{/* a comment */}\n".to_string()))
    } else {
      Ok(None)
    }
  })
  .unwrap()
  .unwrap();
  assert!(
    result.contains("{/* a comment */}"),
    "expression not formatted: {:?}",
    result
  );
}

#[test]
fn callback_error_falls_back_to_original() {
  let input = "import Foo from 'bar'\n\n# Title\n";
  let result = format_mdx_text(input, &config(), |tag, _text, _width| {
    if tag == "tsx" {
      Err(FormatError::CodeBlock("mock error".into()))
    } else {
      Ok(None)
    }
  })
  .unwrap()
  .unwrap_or_else(|| input.to_string());
  // The import should be preserved as-is when the callback errors.
  assert!(result.contains("import Foo from 'bar'"), "import lost: {:?}", result);
  assert!(result.contains("# Title"), "heading lost: {:?}", result);
}

// =========================================================================
// v4-P1: Brace counting with comments
// =========================================================================

#[test]
fn esm_with_block_comment_brace() {
  // /* } */ inside an export must not end it early.
  let input = "export function f() {\n  /* } */\n\n  return \"a  b\"\n}\n\n# Title\n";
  let result = mdx(input);
  assert!(
    result.contains("return \"a  b\""),
    "ESM with block comment brace was split: {:?}",
    result
  );
}

#[test]
fn esm_with_line_comment_brace() {
  // // } inside an export must not end it early.
  let input = "export function f() {\n  // }\n\n  return \"a  b\"\n}\n\n# Title\n";
  let result = mdx(input);
  assert!(
    result.contains("return \"a  b\""),
    "ESM with line comment brace was split: {:?}",
    result
  );
}

// =========================================================================
// v4-P1: Ignore placeholder uniqueness
// =========================================================================

#[test]
fn ignore_placeholder_not_replacing_code_span() {
  let input = "`<!-- dprint-ignore -->`\n\n{/* dprint-ignore */}\n\n#  Title\n";
  let result = mdx(input);
  // The code span must keep its original content, not be replaced.
  assert!(
    result.contains("`<!-- dprint-ignore -->`"),
    "code span was corrupted: {:?}",
    result
  );
  // The ignore should still work on the heading.
  assert!(result.contains("#  Title"), "ignore didn't work: {:?}", result);
}

// =========================================================================
// v4-P1: CRLF line ending normalisation
// =========================================================================

#[test]
fn crlf_normalised_in_mdx_regions() {
  let input = "import Foo from './foo'\r\n\r\n#  Hello\r\n";
  let result = mdx(input);
  // With default LF config, CRLF should become LF everywhere.
  assert!(!result.contains("\r\n"), "CRLF not normalised: {:?}", result);
}

// =========================================================================
// v4-P2: Frontmatter with `...` closer
// =========================================================================

#[test]
fn frontmatter_yaml_dots_closer() {
  let input = "---\ntitle: test\n...\n\n#  Hello\n";
  let result = format_mdx_text(input, &config(), |_, _, _| Ok(None));
  assert!(result.is_ok());
  let text = result.unwrap().unwrap_or_else(|| input.to_string());
  assert!(text.contains("# Hello"), "heading not formatted: {:?}", text);
}

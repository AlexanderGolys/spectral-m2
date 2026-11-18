; The injections of tree-sitter-md that Markdown notes use, written for
; tree-sitter-highlight: Neovim injects the whole text of a node, but
; tree-sitter-highlight leaves out its children unless told to take them in.

(fenced_code_block
  (info_string
    (language) @injection.language)
  (code_fence_content) @injection.content)

((inline) @injection.content
  (#set! injection.language "markdown_inline")
  (#set! injection.include-children))

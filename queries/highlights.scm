; Comments
[
  (line_comment)
  (block_comment)
] @comment

; Literals
(integer_literal) @number

(float_literal) @number.float

[
  (string_literal)
  (raw_string_literal)
] @string

(escape_sequence) @character.special

(symbol) @variable

; The quoted name is the expression's sole named child. Avoid its field name here
; so highlighting remains compatible across the symbol -> token field rename.
(quote_expression (_) @variable)

; Operators
; Operator-bearing nodes keep this field across the assignment/structured-
; binding CST split, so avoid coupling the base capture to every node name.
(_ operator: _ @operator)

; Punctuation
[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "<|"
  "|>"
] @punctuation.bracket

[
  ","
  ";"
] @punctuation.delimiter



[
  "if"
  "else"
  "then"
  "when"
  "list"
  "do"
] @keyword.conditional

[
  "for"
  "while"
  "in"
  "from"
  "to"
] @keyword.repeat

[
  "return"
  "break"
  "continue"
] @keyword.return

[
  "try"
  "catch"
  "throw"
  "trap"
  "except"
  "shield"
] @keyword.exception

[
  "global"
  "local"
  "symbol"
  "threadVariable"
  "threadLocal"
] @keyword.modifier

[
  "time"
  "timing"
  "elapsedTime"
  "elapsedTiming"
  "profile"
  "TEST"
  "breakpoint"
  "finish"
  "step"
] @keyword.debug

(debug_clause
  keyword: _ @keyword.debug)

[
  "and"
  "or"
  "xor"
  "not"
] @keyword.operator

[
  "new"
  "of"

] @keyword

((binary_expression
  left: [
    (integer_literal)
    (float_literal)
  ]
  operator: "_"
  right: (symbol)) @number
  (#set! priority 151))

; Function parameters
(lambda_expression
  parameters: [
    (symbol) @variable.parameter
    (parenthesized_expression
      (symbol) @variable.parameter)
    (sequence
      (symbol) @variable.parameter)
    (list
      (symbol) @variable.parameter)
    (array
      (symbol) @variable.parameter)
    (angle_bar_list
      (symbol) @variable.parameter)
  ])

; Function definitions
[
  (assignment
    left: (symbol) @function
    right: (lambda_expression))
  (local_assignment
    left: (symbol) @function
    right: (lambda_expression))
]


; Members, options, and properties
(binary_expression
  operator: ["." ".?" "#" "#?" "_"]
  right: [(symbol) (integer_literal)] @property)



; Types
(new_statement
  class: (_) @type
  "of" @keyword
  parent: (_) @type)

(new_statement
  "from" @keyword)

(binary_expression
  left: (symbol) @function.call
  operator: "SPACE")

(new_statement
  class: _ @type)

; Method installations
; Named methods with a single, unparenthesized domain type.
((_
  left: (binary_expression
    left: (symbol) @label
    operator: "SPACE"
    right: (symbol) @type.parameter)
  operator: [
    "="
    ":="
  ] @keyword.operator)
  (#match? @label "^[a-z].*"))

; An unnamed adjacency method has no visible sign to label.
((_
  left: (binary_expression
    left: (symbol) @type.parameter @_first-type
    operator: "SPACE"
    right: (symbol) @type.parameter)
  operator: [
    "="
    ":="
  ] @keyword.operator)
  (#not-match? @_first-type "^[a-z].*"))

; Infix operator methods.
((_
  left: (binary_expression
    left: (_) @type.parameter
    operator: _ @label
    right: (_) @type.parameter)
  operator: [
    "="
    ":="
  ] @keyword.operator)
  (#not-eq? @label "")
  (#not-any-of? @label "SPACE" "." ".?" "#" "#?" "_"))

; A _ B := (x, y) -> x*y. Requiring a function-shaped implementation
; avoids treating an ordinary indexed assignment as an installation.
(_
  left: (binary_expression
    left: (_) @type.parameter
    operator: "_" @label
    right: (_) @type.parameter)
  operator: ["=" ":="] @keyword.operator
  right: [
    (lambda_expression)
    (option
      operator: "=>")
  ])

; Named methods with parenthesized domain types. Delimiters deliberately keep
; their ordinary punctuation captures.
(_
  left: (binary_expression
    left: (symbol) @label
    operator: "SPACE"
    right: [
      (sequence
        (symbol) @type.parameter)
      (parenthesized_expression
        (symbol) @type.parameter)])
  operator: [
    "="
    ":="
  ] @keyword.operator)

; - ZZ := x -> -x
(_
  left: (prefix_expression
    operator: _ @label
    operand: (symbol) @type.parameter)
  operator: [
    "="
    ":="
  ] @keyword.operator)

; ZZ ! := n -> if n > 0 then n*(n-1)! else 1
(_
  left: (postfix_expression
    operand: (symbol) @type.parameter
    operator: _ @label)
  operator: [
    "="
    ":="
  ] @keyword.operator)

; A typical value is part of the installed signature. Highlight its arrow like
; the installation operator and its value like the domain types.
((_
  left: (binary_expression
    operator: _ @_installation-sign)
  operator: ["=" ":="]
  right: (option
    left: (symbol) @type.parameter
    operator: "=>" @keyword.operator))
  (#not-any-of? @_installation-sign "." ".?" "#" "#?"))

(_
  left: [
    (prefix_expression)
    (postfix_expression)
    (new_statement)
  ]
  operator: ["=" ":="]
  right: (option
    left: (symbol) @type.parameter
    operator: "=>" @keyword.operator))

((binary_expression
  left: (symbol) @function.builtin
  operator: "SPACE"
  right: (sequence
    (quote_expression
      (_) @label)
    .
    (_) @type.parameter
    .
    (_) @type.parameter))
  (#eq? @function.builtin "installAssignmentMethod"))

; Builtins
((symbol) @variable.builtin
  (#match? @variable.builtin "^((o[1-9][0-9]*)|oo|ooo|oooo)$"))

((symbol) @constant.builtin
  (#any-of? @constant.builtin "CatalanConstant" "EulerConstant" "ii" "pi" "null" "infinity"))

((symbol) @boolean
  (#any-of? @boolean "true" "false"))

((symbol) @error
  (#any-of? @error "error" "stderr")
  (#set! priority 160))



((symbol) @variable.builtin
  (#any-of? @variable.builtin
    "allowableThreads" "debugLevel" "defaultPrecision" "engineDebugLevel" "errorDepth" "gbTrace"
    "interpreterDepth" "lineNumber" "loadDepth" "maxAllowableThreads" "maxExponent" "minExponent"
    "numTBBThreads" "printingAccuracy" "printingLeadLimit" "printingPrecision" "printingTimeLimit"
    "printingTrailLimit" "version" "printWidth" "recursionLimit" ))

; Special strings
((string_literal) @string.special.url
  (#match? @string.special.url "^http[s]?://.*"))

((string_literal) @string.special.url
  (#match? @string.special.url "^www\\..*"))

((binary_expression
  left: (symbol) @function.builtin
  operator: "SPACE"
  right: [
    (string_literal) @string.special.url
    (parenthesized_expression
      .
      (string_literal) @string.special.url)
    (sequence
      .
      (string_literal) @string.special.url)
  ])
  (#any-of? @function.builtin "splitWWW" "getWWW" "urlEncode"))

; Packages
((binary_expression
  left: (symbol) @function
  operator: "SPACE"
  right: [
    (symbol) @module
    (parenthesized_expression . (symbol) @module)
    (sequence . (symbol) @module)
    (string_literal) @module
    (parenthesized_expression . (string_literal) @module)
    (sequence . (string_literal) @module)
  ])
  (#any-of? @function
    "loadPackage" "installPackage" "uninstallPackage" "needsPackage" "endPackage"
    "newPackage" ))

((binary_expression
  left: (symbol) @function
  operator: "_"
  right: (symbol) @module)
  (#any-of? @function "importFrom" "exportFrom"))

((binary_expression
  left: (symbol) @function
  operator: "SPACE"
  right: [(sequence
              (string_literal) @namespace)
          (string_literal) @namespace]
  ) (#eq? @function "load"))

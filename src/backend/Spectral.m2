-- -*- coding: utf-8 -*-
newPackage(
	"Spectral",
	Version => "0.1",
	Headline => "drawing values in the spectral-m2 shell",
	PackageImports => {"Complexes"},
	Authors => {{
		Name => "Alexander Golys",
		Email => "alexandergolys@gmail.com",
		HomePage => "https://github.com/AlexanderGolys/spectral-m2"
	}},
	DebuggingMode => false
)

export {"SpecNet", "specNet", "specType", "specType'", "speek", "speek'", "UnderExpression"}

integerText := lookup(toString, ZZ);
typeText := lookup(toString, Type);
SpecNet = new Type of HashTable
UnderExpression = new HeaderType of Expression
specNet = method(Dispatch => Thing, TypicalValue => SpecNet)
specType = method(Dispatch => Thing, TypicalValue => SpecNet)
specType' = method(TypicalValue => SpecNet)
specType Thing := x -> specType'(1, x)
-- Class objects are encoded as their ancestry by the shell's frame encoder.
node := (type, children, text) -> new SpecNet from {
    "classes" => type, "children" => toList children, "text" => text
}
withMetadata := (drawing, fields) -> new SpecNet from merge(drawing, hashTable fields, (a,b) -> b)
leaf := x -> (
    drawing := node(class x, {}, if instance(x, String) then x else try toString x else typeText class x);
    if instance(x, Ring) or instance(x, RingFamily) then
        withMetadata(drawing, {"tex" => (try texMath x)})
    else drawing
)
activeValues := {}
forgetDrawing = () -> activeValues = {}
traverse := (x, draw) -> (
    if any(activeValues, previous -> previous === x) then return leaf x;
    activeValues = append(activeValues, x);
    result := try draw x else leaf x;
    activeValues = drop(activeValues, -1);
    result
)


-- Formatting follows M2's expression constructors; opaque holders are leaves.
specNet SpecNet := identity
specNet Holder := x -> leaf x#0
specNet Thing := x -> specNet expression x
specNet InexactNumber := leaf
specNet RingElement := x -> specNet expression x
specNet BasicList := x -> traverse(x, items ->
    node(class items, apply(toList items, specNet), null))
specNet Expression := x -> node(class x, apply(toList x, specNet), null)
specNet MatrixExpression := x -> node(class x,
    apply(toList x, row -> if instance(row, Option) then specNet row
        else node(List, apply(toList row, specNet), null)), null)
specNet IndexedVariable := x -> node(Subscript, apply(toList x, specNet), null)
specNet Complex := complex -> (
    if complex == 0 then return specNet 0;
    (lo, hi) := concentration complex;
    drawing := UnderExpression{expression complex_lo, lo};
    scan(lo+1..hi, degree -> drawing = MapExpression{
        drawing,
        UnderExpression{expression complex_degree, degree},
        new Array from {Table entries complex.dd_degree}
    });
    specNet drawing
)

-- Ordinary tables expose their pairs; mathematical table subclasses keep
-- their own expression or opaque formatter.
tableDrawing := (base, x) -> (
    shape := expression x;
    if not instance(shape, Holder) then return specNet shape;
    if lookup(expression, class x) =!= lookup(expression, base)
        or lookup(net, class x) =!= lookup(net, base) then return leaf x;
    traverse(x, table -> node(class table, apply(pairs table, entry ->
        node(Option, apply(toList entry, specNet), null)), null))
)
specNet HashTable := tableDrawing_HashTable
specNet MutableHashTable := tableDrawing_MutableHashTable

sourceTarget := position -> (
	file := position#0;
	candidates := if #file == 0 then {} else
			select(join({file}, apply(path, directory -> directory | file)), fileExists);
	if #candidates != 0 then
		urlEncode("file://" | toAbsolutePath first candidates
				| "#L" | integerText position#1 | ":C" | integerText position#2
				| (if #position > 3 then
							"-L" | integerText position#3 | ":C" | integerText position#4
						else ""))
)


hypertextType := value Core#"private dictionary"#"Hypertext"
exampleWidth := value Core#"private dictionary"#"ArgPrintWidthN"
-- A link to another node points at the page M2 writes it to, named by
-- `toFilename` of the node's key. Read back through the same encoding, each
-- such link instead carries the key as an M2 string, which is what the shell
-- asks `help` for when the link is clicked. Links elsewhere stay.
toFilename := value Core#"private dictionary"#"toFilename"

filenameCodes := hashTable apply(1..127, i -> (substring(toFilename ascii i, 1), ascii i))

nodeKey := name -> (
	codes := characters substring(name, 1);
	key := {};
	i := 0;
	while i < #codes do (
		code := if codes#i == "_" then
				concatenate codes_{i..min(i + 2, #codes - 1)}
			else codes#i;
		if not filenameCodes#?code then
			return null;
		key = append(key, filenameCodes#code);
		i += #code
	);
	concatenate key
)
escapeAttribute := text -> replace(">", "&gt;", replace("<", "&lt;",
	replace("\"", "&quot;", replace("&", "&amp;", text))))
linkedKeys := page -> (
	pattern := ///href="(?:[^"]*/)?(_[^"/]*)\.html"///;
	linked := "";
	while (match := regex(pattern, page)) =!= null do (
		key := nodeKey substring(match#1, page);
		linked = linked | substring(page, 0, match#0#0)
			| (if key === null then
						substring(match#0, page)
					else "href=\"" | escapeAttribute format key | "\"");
		page = substring(page, match#0#0 + match#0#1)
	);
	linked | page
)
-- A source position is linked as `file:///dir/name.m2#L1:C2-L3:C4` and spelled
-- with its whole directory, which is most of a line and none of what is wanted.
-- Its link stays, and its text keeps the file's name, one directory above it,
-- and the position; any directories further up are folded into `[...]/`.
foldedPath := path -> (
	parts := select(separate("/", path), part -> part != "");
	parents := drop(parts, -1);
	if #parents == 0 then
		last parts
	else if #parents == 1 then
		(if first path == "/" then
					"/"
				else "") | concatenate between("/", parts)
	else "[...]/" | concatenate between("/", parts_{-2, -1})
)
shortenedPaths := page -> (
	pattern := ///(<a href="file://([^"#]*)[^"]*">)([^<]*)</a>///;
	shortened := "";
	while (match := regex(pattern, page)) =!= null do (
		path := substring(match#2, page);
		text := substring(match#3, page);
		shortened = shortened | substring(page, 0, match#0#0) | substring(match#1, page)
			| (if substring(text, 0, #path) == path then
						foldedPath path | substring(text, #path)
					else text) | "</a>";
		page = substring(page, match#0#0 + match#0#1)
	);
	shortened | page
)
linkedSources := page -> (
	pattern := ///<span class="tt">([^<]*\.m2):([0-9]+):([0-9]+)</span>///;
	linked := "";
	while (match := regex(pattern, page)) =!= null do (
		file := replace("&amp;", "&", replace("&gt;", ">", replace("&lt;",
			"<", substring(match#1, page))));
		candidates := select(join({file}, apply(path, directory -> directory
				| file)), fileExists);
		original := substring(match#0, page);
		target := if #candidates != 0 then
				urlEncode("file://" | toAbsolutePath first candidates | "#L"
						| substring(match#2, page) | ":C" | substring(match#3, page));
		linked = linked | substring(page, 0, match#0#0)
			| (if target === null then
						original
					else
						"<a href=\"" | escapeAttribute target | "\">"
					| original | "</a>");
		page = substring(page, match#0#0 + match#0#1)
	);
	linked | page
)

specNet FilePosition := x -> (
    text := (if #x#0 == 0 then "" else foldedPath x#0)
        | ":" | integerText x#1 | ":" | integerText x#2
        | (if #x > 3 then "-" | integerText x#3 | ":" | integerText x#4 else "");
    withMetadata(node(class x, {}, text), {"target" => sourceTarget x})
)
operators := unique join(
    value Core#"private dictionary"#"binaryOperators",
    value Core#"private dictionary"#"prefixOperators",
    value Core#"private dictionary"#"postfixOperators"
)
specNet Keyword := x -> withMetadata(leaf x, {
    "highlight" => if member(x, operators) then {"Operator"} else Keyword
})
specNet Function := x -> (
    position := locate x;
    withMetadata(leaf x, {"target" => if position =!= null then
				sourceTarget position})
)
specNet hypertextType := x -> withMetadata(node(class x, {}, toString net x), {
    "html" => shortenedPaths linkedSources linkedKeys html x,
    "typeNames" => unique flatten apply(dictionaryPath, dictionary ->
        select(keys dictionary, name -> instance(value dictionary#name, Type))),
    "exampleWidth" => exampleWidth
})

-- Types are a bounded fold of the value tree; leaves contribute their class.
foldType := (depth, drawing) -> (
    if depth <= 1 or drawing#"text" =!= null then specNet drawing#"classes"
    else node(RowExpression, {
        specNet drawing#"classes",
        node(Array, apply(drawing#"children", child -> foldType(depth-1, child)), null)
    }, null)
)
specType' (ZZ, Thing) := (depth, x) -> (
    if depth <= 1 then specNet class x else foldType(depth, specNet x)
)

-- Presentation metadata is attached only at the transport boundary.
drawnAs = x -> try withMetadata(specNet x, {
    "fallback" => (try toString net x), "code" => (try toString x)
}) else leaf x
typedAs = x -> try specType x else leaf class x

speek' = method()
speek' (ZZ, Thing) := Net => (depth, x) -> (
    drawing := specNet x;
    type := drawing#"classes";
    if drawing#"text" =!= null then net (type => drawing#"text")
    else if depth == 0 then net (toString type | "{...}")
    else net (type => apply(drawing#"children", child -> speek'(depth-1, child)))
)
speek = method(Dispatch => Thing)
speek Thing := Net => x -> speek'(2, x)
net SpecNet := speek

beginDocumentation()
doc ///
Node
  Key
    Spectral
  Headline
    structured value and type formatting
  Description
    Text
      @TO specNet@ preserves expression structure and the classes of components.
      @TO specType@ independently formats the type of a value.
  Subnodes
    SpecNet
    specNet
    specType
    speek
    UnderExpression
Node
  Key
    UnderExpression
  Headline
    center a label beneath an expression
  Usage
    UnderExpression{base, label}
  Description
    Text
      Keeps the baseline of the base while placing the label below it.
      Complexes compose this with @TO MapExpression@ to show module degrees.
Node
  Key
    SpecNet
  Headline
    transmissible expression structure with component classes
  Description
    Text
      A SpecNet contains a class, ordered children, and optional terminal text.
      Terminal text replaces children for an opaque value. Class objects are
      transmitted as their ancestry; the original value is never transmitted.
Node
  Key
    specNet
    (specNet, Thing)
  Headline
    format a value without flattening its expression
  Usage
    specNet x
  Description
    Text
      The default method uses @TO expression@, traversing its structure and
      preserving each component's class. A @TO Holder@ becomes an opaque leaf.
      Install @TO expression@ for a new type to compose existing expression
      shapes, or install specNet to customize its transmissible representation.
Node
  Key
    specType
    (specType, Thing)
    specType'
    (specType', ZZ, Thing)
  Headline
    format structured type information for a value
  Usage
    specType x
    specType'(depth, x)
  Description
    Text
      Receives the original value. The depth is a non-negative integer, default 1.
      Depth one gives the outer type. Each further level folds one layer of
      components to their types. Containers retain positional element types;
      the same expression tree supplies the constructor and component types.
Node
  Key
    speek
    (speek, Thing)
    speek'
    (speek', ZZ, Thing)
  Headline
    inspect structured formatting
  Usage
    speek x
    speek'(depth, x)
///
end--

-- m2-ls: disable E19
-- -*- coding: utf-8 -*-
newPackage(
	"Spectral",
	Version => "0.1",
	Headline => "drawing values in the spectral-m2 shell",
	PackageImports => {"Complexes", "Varieties", "Text"},
	Authors => {{
		Name => "Alexander Golys",
		Email => "alexandergolys@gmail.com",
		HomePage => "https://github.com/AlexanderGolys/spectral-m2"
	}},
	DebuggingMode => false
)

export {
	"SpecNet",
	"specNet",
	"specType",
	"specType'",
	"speek",
	"speek'",
	"UnderExpression",
	"typeDepth",
	"valueDepth",
	"specNet'"
}

-- The print width M2 runs documentation examples at. Core keeps it to itself,
-- and archived example output cannot be unwrapped without it.
exampleWidth := value Core#"private dictionary"#"ArgPrintWidthN"

SpecNet = new Type of HashTable
UnderExpression = new HeaderType of Expression
-- A part a drawing introduces for a structure, as {name, part, shape}: spelled
-- by its name until the value depth reaches it, then drawn from its shape.
Intermediate = new HeaderType of Expression

specNet = method(Dispatch => Thing, TypicalValue => SpecNet)
specType = method(Dispatch => Thing, TypicalValue => SpecNet)
specType' = method(TypicalValue => SpecNet)
specNet' = method(TypicalValue => SpecNet)
speek = method(Dispatch => Thing)
speek' = method()

opaque = method(Dispatch => Thing, TypicalValue => SpecNet)
spelling = method(Dispatch => Thing, TypicalValue => String)
sourceTarget = method(Dispatch => Thing)
documentationTags = method(Dispatch => Thing)
attributeText = method()
foldedPath = method()
valueClass = method()
configuredDepth = method()
placeholder = method(Dispatch => Thing)
parameters = method(Dispatch => Thing, TypicalValue => List)
spelledShape = method()

Thing.typeDepth = 1
Module.typeDepth = 2
Thing.valueDepth = 1
-- A complex shows its modules and differentials rather than only their names.
Complex.valueDepth = 2

-- The nearest setting of `depth` in the ancestry of x's class.
configuredDepth (Symbol, Thing) := (depth, x) -> (
	owner := first select(ancestors class x, ancestor -> ancestor#?depth);
	if not instance(owner#depth, ZZ) or owner#depth < 0 then
		error(toString depth | " must be a non-negative integer");
	owner#depth
)

-- Structures whose kind goes by a conventional name are what the value depth
-- counts: past it an unnamed one keeps only that name. Anything else, such as
-- a ring element, is always drawn whole, and so is a variety's structure sheaf,
-- which is spelled by its variety.
placeholder Thing := x -> null
placeholder Ring := x -> "R"
placeholder SheafOfRings := x -> null
placeholder Ideal := x -> "I"
placeholder Module := x -> "M"
placeholder Variety := x -> "X"
placeholder CoherentSheaf := x -> "F"
placeholder Complex := x -> "C"

-- The values a structure is built over, which its expression spells in place:
-- modules and sheaves are over their ring, a sheaf of rings over its variety.
parameters Thing := x -> {}
parameters Ideal := parameters Module := parameters Matrix :=
parameters Complex := parameters CoherentSheaf := parameters SheafMap :=
parameters Variety := x -> {ring x}
parameters QuotientRing := x -> {ambient x}
parameters PolynomialRing := x -> {coefficientRing x}
parameters SheafOfRings := x -> {variety x}

-- What a value is drawn from. The rank of a module or sheaf is an index on it,
-- as in OO_X^2, rather than a power, and M2 spells rank one as an explicit
-- `^1` that its type already tells apart from the base.
spelledShape (Thing, Thing) := (x, shape) -> shape
spelledShape (Module, Expression) := spelledShape (CoherentSheaf,
	Expression) := (x, shape) -> (
		if instance(shape, Superscript) and instance(shape#1, OneExpression) then
			shape#0
		else if instance(shape, Adjacent) then
			apply(shape, part -> spelledShape(x, part))
		else shape
	)

-- A drawing is a class laid out over ordered children; an opaque one is spelled
-- by its text instead. Further fields annotate it: `drawing ++ {Tex => …}`.
new SpecNet from Sequence := (SpecNet, shape) -> hashTable {
	Classes => shape#0,
	Children => toList shape#1,
	symbol Text => null
}
SpecNet ++ List := SpecNet => (drawing, fields) -> new SpecNet from merge(drawing,
	hashTable fields, last@@splice)

-- What a drawing stands for: its value's class, or for a component that is only
-- part of an expression, the expression's own.
valueClass SpecNet := drawn -> if drawn.?Type then drawn.Type else drawn.Classes

-- Values whose expression holds nothing but themselves.
opaque Thing := x -> new SpecNet from {
	Classes => class x,
	symbol Type => class x,
	Children => {},
	symbol Text => spelling x
}
-- A name stands for its value.
opaque Symbol := name -> (lookup(opaque, Thing)) name ++ {symbol Type => class value name}
opaque Ring := opaque RingFamily := opaque ScriptedFunctor := x -> (
	(lookup(opaque, Thing)) x ++ {Tex => texMath x}
)

spelling Thing := x -> try toString x else toString class x
spelling String := identity

-- A source position is linked by its file, found on the load path, and spelled
-- with the file's name and one directory above it.
sourceTarget FilePosition := position -> (
	file := position#0;
	found := select(prepend(file, apply(path, directory -> directory | file)),
		fileExists);
	if #file > 0 and #found > 0 then
		urlEncode concatenate(
			"file://", toAbsolutePath first found,
			"#L", toString position#1, ":C", toString position#2,
			if #position > 3 then
				("-L", toString position#3, ":C", toString position#4) else ""
		)
)
foldedPath String := path -> (
	parts := select(separate("/", path), part -> part != "");
	parents := drop(parts, -1);
	if #parents == 0 then
		last parts
	else if #parents == 1 then
		(if first path == "/" then
					"/"
				else "") | demark("/", parts)
	else "[...]/" | demark("/", take(parts, -2))
)
spelling FilePosition := position -> concatenate(
	foldedPath position#0, ":", toString position#1, ":", toString position#2,
	if #position > 3 then
		("-", toString position#3, ":", toString position#4) else ""
)

-- Documentation is sent as M2 writes it, except that a link to a node carries
-- the node's key as the M2 string `help` reads, and a source position links to
-- its file under a shortened spelling.
documentationTags Thing := List => x -> {}
documentationTags Hypertext := List => page -> unique flatten apply(toList page,
	documentationTags)
documentationTags TO := documentationTags TO2 := documentationTags TOH :=
List => link -> {link#0}

attributeText String := text -> fold((escaped, entity) ->
		replace(entity#0, entity#1, escaped), text,
	{("&", "&amp;"), ("\"", "&quot;"), ("<", "&lt;"), (">", "&gt;")})

-- M2 links a tag wherever it appears with the same target, so the target its
-- own link gets is the one to replace.
keysLinked = method()
keysLinked (String, Hypertext) := (page, document) -> fold((linked, tag) -> (
	anchor := html TO {tag};
	target := regex(///href="[^"]*"///, anchor);
	if target === null then
		linked
	else demark("href=\"" | attributeText format format tag | "\"",
		separate(regexQuote substring(target#0, anchor), linked))
), page, documentationTags document)

-- `file:///dir/name.m2#L1:C2` is spelled with its whole directory, which is
-- most of a line and none of what is wanted; its text keeps the file's name,
-- one directory above it, and the position.
pathsShortened = method()
pathsShortened String := page -> (
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

-- A source position set as code links to its file, found on the load path.
sourcesLinked = method()
sourcesLinked String := page -> (
	pattern := ///<span class="tt">([^<]*\.m2):([0-9]+):([0-9]+)</span>///;
	linked := "";
	while (match := regex(pattern, page)) =!= null do (
		file := replace("&amp;", "&", replace("&gt;", ">", replace("&lt;", "<",
			substring(match#1, page))));
		target := sourceTarget new FilePosition from {file,
			value substring(match#2, page), value substring(match#3, page)};
		original := substring(match#0, page);
		linked = linked | substring(page, 0, match#0#0)
			| (if target === null then
						original
					else "<a href=\"" | attributeText target | "\">" | original | "</a>");
		page = substring(page, match#0#0 + match#0#1)
	);
	linked | page
)

-- Values being drawn: a container that holds itself is drawn once.
drawing := new MutableList
-- How many more values may be unfolded inside the one being drawn; unset
-- outside a drawing.
unfoldable := null
forgetDrawing = () -> (drawing = new MutableList; unfoldable = null)
contentsOf := (container, items) -> (
	if any(toList drawing, ancestor -> ancestor === container) then
		return opaque container;
	drawing#(#drawing) = container;
	budget := unfoldable;
	result := try new SpecNet from (class container, apply(items, specNet))
		else opaque container;
	unfoldable = budget;
	drawing = new MutableList from drop(toList drawing, -1);
	result
)
-- Draw x from `shape`, its expression with each parameter spelled in place
-- given back as the parameter itself, so that it is drawn at the next depth.
unfolding := (x, shape) -> (
	if unfoldable === null then
		return specNet'(configuredDepth(symbol valueDepth, x), x);
	name := placeholder x;
	if name =!= null and unfoldable <= 0 then
		return opaque x ++ {
			Classes => Symbol,
			symbol Text => name
		};
	held := fold((drawn, parameter) -> (
		spelled := expression parameter;
		substituted := part -> if part === spelled then
				Holder {parameter}
			else if instance(part, Expression) and not instance(part, Holder) then
			apply(part, substituted)
		else part;
		substituted drawn
	), shape, parameters x);
	if name =!= null then
		unfoldable -= 1;
	result := specNet held ++ {symbol Type => class x};
	if name =!= null then
		unfoldable += 1;
	result
)

specNet' (ZZ, Thing) := (depth, x) -> (
	outer := unfoldable;
	unfoldable = depth;
	result := specNet x;
	unfoldable = outer;
	result
)

-- A value is drawn as what its expression is made of, down to expressions
-- that hold nothing but the value itself; a container's items are drawn at
-- the container's own depth.
specNet Thing := x -> (
	shape := expression x;
	if instance(shape, Holder) and shape#0 === x then
		opaque x
	else if instance(shape, Holder) and instance(shape#0, Symbol) then
		specNet shape#0
	else unfolding(x, spelledShape(x, shape))
)
specNet SpecNet := identity
specNet Holder := held -> specNet held#0
specNet Expression := shape -> new SpecNet from (class shape, apply(
	toList shape, specNet
))
specNet Parenthesize := lookup(specNet, Expression)
specNet InexactNumber := opaque
specNet VisibleList := specNet MutableList := specNet Option := items ->
	contentsOf(items, toList items)
-- A table keeping the stock formatting shows its pairs.
specNet HashTable := table -> (
	stock := base -> lookup(expression, class table) === lookup(expression, base)
		and lookup(net, class table) === lookup(net, base);
	if any({HashTable, MutableHashTable}, stock) then
		contentsOf(table, apply(pairs table, pair -> pair#0 => pair#1))
	else (lookup(specNet, Thing)) table
)
specNet MatrixExpression := matrix -> new SpecNet from (class matrix,
	apply(toList matrix, row -> if instance(row, Option) then
				specNet row
			else new SpecNet from (List, apply(toList row, specNet))))
specNet IndexedVariable := variable -> new SpecNet from (Subscript,
	apply(toList variable, specNet))
specNet Intermediate := part -> (
	(name, component, shape) := toSequence part;
	if unfoldable =!= null and unfoldable <= 0 then
		specNet name ++ {symbol Type => class component}
	else specNet shape ++ {symbol Type => class component}
)
-- A complex is its chain of modules C_i and differentials d_i between zeros.
specNet Complex := complex -> (
	if complex == 0 then
		return specNet 0;
	(lo, hi) := concentration complex;
	module := degree -> Intermediate {
		Subscript {symbol C, degree},
		complex_degree,
		UnderExpression {Holder {complex_degree}, degree}
	};
	differential := degree -> Intermediate {
		Subscript {symbol d, degree},
		complex.dd_degree,
		new Array from {Table entries complex.dd_degree}
	};
	chain := fold((drawn, degree) -> MapExpression {
		drawn,
		module degree,
		differential degree
	}, MapExpression {expression 0, module lo}, toList(lo + 1 .. hi));
	unfolding(complex, MapExpression {chain, expression 0})
)
specNet FilePosition := position -> opaque position ++ {
	symbol Text => spelling position,
	Target => sourceTarget position
}
specNet Keyword := keyword -> opaque keyword ++ {
	Highlight => if operatorAttributes#?keyword then {"Operator"} else Keyword
}
specNet Function := function -> opaque function ++ {
	Target => (position := locate function;
		if position =!= null then
			sourceTarget position)
}
specNet Hypertext := page -> opaque page ++ {
	symbol Text => toString net page,
	Html => sourcesLinked pathsShortened keysLinked(html page, page),
	TypeNames => unique flatten apply(dictionaryPath, dictionary ->
			select(keys dictionary, name -> instance(value dictionary#name, Type))),
	ExampleWidth => exampleWidth
}

-- Types are a bounded fold of the value's drawing; leaves contribute their class.
specType Thing := x -> specType'(configuredDepth(symbol typeDepth, x), x)
-- Each level of a type below the outermost folds one level of the value.
specType' (ZZ, Thing) := (depth, x) -> (
	if depth <= 1 then
		specNet class x
	else specType'(depth, specNet'(depth - 1, x))
)
specType' (ZZ, SpecNet) := (depth, drawn) -> (
	if depth <= 1 or drawn.Text =!= null then
		specNet valueClass drawn
	else new SpecNet from (RowExpression, {
		specNet valueClass drawn,
		new SpecNet from (Array, apply(drawn.Children,
			child -> specType'(depth - 1, child)))
	})
)
-- Mathematical types have their parameters rather than expression components:
-- a sheaf is a sheaf of modules over its sheaf of rings, which is over the
-- variety, and each type level below the outermost unfolds the parameter once.
scan({Module, Matrix, SheafOfRings, CoherentSheaf, SheafMap}, type ->
		specType' (ZZ, type) := (depth, x) -> (
			if depth <= 1 then
				specNet class x
			else new SpecNet from (RowExpression, {
				specNet class x,
				new SpecNet from (Array, apply(parameters x,
					parameter -> specNet'(depth - 2, parameter)))
			})
		))

-- Presentation metadata is attached only at the transport boundary.
drawnAs = x -> try specNet x ++ {
	Fallback => (try toString net x),
	Code => (try toString x)
} else opaque x
typedAs = x -> try specType x else opaque class x

speek' (ZZ, Thing) := (depth, x) -> speek'(depth, specNet x)
speek' (ZZ, SpecNet) := Option => (depth, drawn) -> valueClass drawn => (
	if drawn.Text =!= null then
		drawn.Text
	else if depth <= 0 then
		"..."
	else apply(drawn.Children, child -> speek'(depth - 1, child))
)
speek Thing := x -> speek'(2, x)
net SpecNet := net@@speek

-- Drawing fields are keys, never values: their symbols stay unset.
scan(values currentPackage#"private dictionary", key ->
		if isMutable key and value key === key then
		protect key)

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
      A SpecNet holds the class that lays it out under Classes, its ordered
      Children, and the symbol Text, whose string replaces the children of an
      opaque value. Optional fields annotate it: Highlight names the semantic
      class to colour it by, Tex its native TeX, Target a source or
      documentation link, and Html, TypeNames and ExampleWidth a documentation
      page. Fallback and Code carry the value's net and input spelling. Class
      objects are transmitted as their ancestry; the original value is never
      transmitted.
Node
  Key
    specNet
    (specNet, Thing)
    specNet'
    (specNet', ZZ, Thing)
  Headline
    format a value without flattening its expression
  Usage
    specNet x
    specNet'(depth, x)
  Description
    Text
      The default method unfolds @TO expression@, preserving each component's
      class, until an expression holds nothing but its value: a named value
      ends at its symbol, so nothing is read from inside rings or varieties.
      Install @TO expression@ for a new type to compose existing expression
      shapes, or install specNet to customize its transmissible representation.
Node
  Key
    valueDepth
  Headline
    control how many unnamed values a drawing unfolds
  Description
    Text
      Assign a non-negative integer to a type's valueDepth field. A value is
      drawn from its expression, and each value unfolded inside another costs
      one level; the items of a list cost none. Past the depth an unnamed value
      keeps only a placeholder: R for a ring, M for a module, X for a variety,
      C for a complex. Named values always keep their name. A complex is a
      chain of its modules C_i and differentials d_i between zeros; at the
      next depth these names give way to the modules and matrices, and each
      further level substitutes one more name.
      Thing.valueDepth is 1 and Complex.valueDepth is 2. Explicit specNet'
      calls use their supplied depth.
Node
  Key
    typeDepth
  Headline
    control the detail in a type annotation
  Description
    Text
      Assign a non-negative integer to a type's typeDepth field. The nearest
      setting in the type ancestry controls specType; subclasses may override it.
      Thing.typeDepth is 1 and Module.typeDepth is 2. Settings may be assigned
      in the shell's init file. Explicit specType' calls use their supplied depth.
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
      Receives the original value. The depth is a non-negative integer.
      specType uses the nearest typeDepth setting in the type's ancestry.
      Thing.typeDepth is 1; Module.typeDepth is 2. Assign these parameters
      in the shell or its init file. Explicit specType' calls override them.
      Modules and matrices use their base ring as a parameter; coherent
      sheaves and sheaf maps use the sheaf of rings they are modules over, and
      a sheaf of rings uses its variety. Each further level unfolds an unnamed
      parameter one step.
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

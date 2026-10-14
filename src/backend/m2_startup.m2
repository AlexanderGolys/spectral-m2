-- spectral-m2 backend: the shell's top-level mode, installed into a real M2.
--
-- This file is loaded with `-e` before M2 enters its own top level, which then
-- reads the whole session from stdin. That is the point: one reader, holding
-- one local dictionary, for the session's lifetime -- exactly what an
-- interactive M2 does. Evaluating each cell through `capture` instead gave
-- every cell a local dictionary of its own, so a top-level `x := 1` bound a
-- symbol the next cell's parse never saw. See the "One Reader Per Session"
-- decision record.
--
-- Two channels, both driven by the shell:
--
--   stdin   one cell, then the terminator statement, per submission.
--   stdout  everything M2 says. The shell hands the child a single pipe for
--           both stdout and stderr, so printed text, diagnostics and the marked
--           frames below arrive in the order M2 produced them -- ordering is the
--           kernel's business rather than something we have to guess at.
--
-- `echoOff` and the mode's lack of an InputPrompt keep the channel free of
-- scaffolding: no echoed input, no `iN :` prompt, no `oN =` labels. Printed text
-- arrives as itself and needs no unpicking.

needsPackage "JSON"

(() -> (
    mode := global SpectralTopLevelMode;

    -- The rendered value and type annotation of the statement in flight. The
    -- hooks below only record them; the terminator sends them, because a
    -- statement that throws or ends in `;` never reaches Print, and the frame
    -- must leave exactly once per cell all the same.
    renderedValue := null;
    typeInfo := null;
    hypertextType := value Core#"private dictionary"#"Hypertext";

    -- Whether the statement in flight ran to completion: the hooks below are the
    -- last thing a statement does, and one that throws runs none of them.
    --
    -- Nothing here asks what a failed statement *managed* to do before it failed,
    -- because there is no answer to that. A throw can land anywhere -- part-way
    -- through a nested parallel assignment, with some of its targets bound and
    -- the rest not -- so any reading of the session's state afterwards is a
    -- guess. This flag is only ever allowed to make a frame say the wrong thing,
    -- never to decide whether a frame is sent.
    completed := false;

    -- What the terminator evaluates to, so the shell's own statement is told
    -- apart by its type rather than by state carried across a statement
    -- boundary. Its hooks are inert: the statement reports nothing, binds no
    -- `oN`, and leaves `oo`/`ooo`/`oooo` alone. The stock AfterEval rotates that
    -- history on every statement, a null one included, so a terminator valued
    -- `null` duplicated `oo` into `ooo` after every cell.
    cellEnd := new Type of BasicList;
    cellEnd#AfterEval = identity;
    cellEnd#{mode, BeforePrint} = identity;
    cellEnd#{mode, NoPrint} = identity;
    cellEnd#{mode, AfterNoPrint} = identity;
    renderAfterPrint := value -> "" | concatenate apply(
        select(deepSplice sequence value, item -> item =!= null),
        item -> toString net item
    );

    Thing#{mode, BeforePrint} = value -> (
        lookupMethod := lookup({Standard, BeforePrint}, class value) ?? identity;
        lookupMethod value
    );
    -- What a value is drawn as, chosen by class. A frame's "value" is the
    -- shell's M2Value (src/backend.rs) in serde's spelling of an enum, a one-key
    -- table naming the variant:
    --
    --     net => text                   tex => {source, fallback}
    --     markup => {html, fallback}    hashtable => {{key, value}, ...}
    --     collection => {kind, items}
    --
    -- A value is either drawn whole by one of M2's own formatters, or it is a
    -- container whose entries are rendered one by one, so the shell can set
    -- each in its own colour. Dispatch takes the whole argument: a Sequence
    -- would otherwise be spread into separate arguments.
    render := method(Dispatch => Thing);
    asNet := value -> hashTable {symbol net => toString net value};
    -- Every class answers `tex`, most of them with their Net in a typewriter
    -- font; one with a `texMath` of its own has a real TeX form. A formatter
    -- that throws costs the value its TeX, never the statement its result.
    drawnWhole := value -> (
        if lookup(texMath, class value) === lookup(texMath, Thing) then
            asNet value
        else try hashTable {
            symbol tex => hashTable {symbol source => tex value, symbol fallback =>
                    toString net value}
        } else asNet value
    );
    render Thing := HashTable => drawnWhole;
    -- M2's own test for whether a function was ever bound to a name.
    hasAttribute := value Core#"private dictionary"#"hasAttribute";
    reverseDictionary := value Core#"private dictionary"#"ReverseDictionary";

    -- A function's TeX is its Net in a typewriter font, and nothing more. A
    -- named one is drawn by its name; an anonymous one's Net only says where it
    -- was written (`FunctionClosure[stdio:2:36-2:42]`), so it is drawn by its
    -- class instead.
    render Function := HashTable => value -> (
        if hasAttribute(value, reverseDictionary) then
            asNet value
        else hashTable {symbol net => toString class value}
    );
    -- Every documentation node at once: DIV, TABLE, UL and the rest all descend
    -- from Hypertext.
    render hypertextType := HashTable => value -> hashTable {
        symbol markup => hashTable {symbol html => html value, symbol fallback =>
                toString net value}
    };
    -- Only what M2 itself draws as a plain table: a class that still uses
    -- HashTable's own `net`. Inheritance alone says far too much -- matrices,
    -- modules, ideals, rings and some fifty more are HashTables that draw
    -- themselves -- and reading M2's `net` dispatch covers every package's
    -- types without listing them.
    table := value -> hashTable {
        symbol hashtable => apply(pairs value, pair -> apply(toList pair, render))
    };
    render HashTable := HashTable => value -> (
        if lookup(net, class value) === lookup(net, HashTable) then
            table value
        else drawnWhole value
    );
    -- The kind names the nearest of the shell's collection kinds, so a list
    -- subclass such as Hybrid arrives as the List it is drawn as.
    collection := (kind, value) -> hashTable {
        symbol collection => hashTable {symbol kind => kind, symbol items => apply(toList value,
            render)}
    };
    render List := HashTable => value -> collection("List", value);
    render Array := HashTable => value -> collection("Array", value);
    render Sequence := HashTable => value -> collection("Sequence", value);
    render AngleBarList := HashTable => value -> collection("AngleBarList", value);
    render VerticalList := HashTable => value -> collection("VerticalList", value);
    -- A mutable container may hold itself, which is why M2 draws one only as a
    -- summary (`MutableHashTable{...3...}`). The first one met is opened anyway;
    -- any met while it is open stays folded, so nesting -- and with it any
    -- self-loop -- ends at the first repeat. Reset with each result as well, so a
    -- render that threw part-way cannot leave every later one folded.
    openingMutable := false;
    openMutable := (value, open) -> (
        if openingMutable then
            return drawnWhole value;
        openingMutable = true;
        rendered := open value;
        openingMutable = false;
        rendered
    );
    render MutableHashTable := HashTable => value -> (
        if lookup(net, class value) === lookup(net, MutableHashTable) then
            openMutable(value, table)
        else drawnWhole value
    );
    render MutableList := HashTable => value -> openMutable(value, entries -> collection("List",
        entries));

    -- One frame closes one input. Marked with a control character no diagnostic
    -- and no prompt can begin with, so the shell can pick frames out of the
    -- stream by inspecting the first byte alone; printed text that happens to
    -- start with it is the user's own doing and reads back as a frame, which is
    -- the same bargain any in-band marker makes.
    frameMark := ascii 29;
    emitFrame := payload -> (
        stdio << frameMark << toJSON payload << endl << flush;
    );
    sendResult := () -> emitFrame hashTable {
        "number" => lineNumber,
        "value" => renderedValue,
        "typeInfo" => typeInfo,
        "hasValue" => OutputDictionary#?("o" | toString lineNumber),
        "depth" => interpreterDepth,
        "error" => not completed
    };
    forgetResult := () -> (
        renderedValue = null;
        typeInfo = null;
        completed = false;
        openingMutable = false;
    );
    recordTypeInfo := value -> (
        annotation := (lookup(AfterPrint, class value)) value;
        typeInfo = renderAfterPrint(annotation ?? class value );
        completed = true;
    );

    -- Recording rather than printing keeps the channel free of everything but
    -- genuinely printed text. A `;` still suppresses the value: NoPrint
    -- deliberately skips `net value`, which is the render the punctuation exists
    -- to avoid on large objects. The type is read either way, so the shell can
    -- say what a suppressed result is without paying to render it.
    --
    -- Nothing deliberately has no Print override: an unterminated statement
    -- reports its result whatever it is, and `net null` is already "null". A `;`
    -- still silences it, since NoPrint stays identity for every class.
    Thing#{mode, Print} = value -> (
        renderedValue = render value;
    );
    Thing#{mode, NoPrint} = identity;
    -- Deliberately AfterPrint in both branches: stock M2 reads AfterNoPrint under
    -- a `;`, which only some classes define, so the type would appear or vanish
    -- with the punctuation. Reading AfterPrint always gives every class its type.
    -- These run last for a statement that completed, which is what makes reaching
    -- them the definition of having completed.
    Thing#{mode, AfterPrint} = recordTypeInfo;
    Thing#{mode, AfterNoPrint} = recordTypeInfo;

    -- Sent by the shell after every cell, and the only part of a submission the
    -- user did not write. It is the sole thing that closes an input.
    --
    -- Sending is unconditional, and that is the whole point. A statement that
    -- throws runs none of the hooks, so something has to close the input or the
    -- shell waits for a frame that never comes; but a terminator that decided
    -- *whether* to send would be reading state the failed statement may have
    -- reached into. Getting that wrong sends two frames for one input, which
    -- misfiles every result after it, or none, which hangs the session until a
    -- timeout. So exactly one frame leaves here per submission by construction,
    -- and the only thing inference is allowed to affect is what the frame says.
    -- The top level carries on after an error -- unlike `capture`, which forces
    -- `stopIfError` -- so this statement runs either way.
    --
    -- Rewinding `lineNumber` costs this statement its own input number, keeping
    -- the shell's `iN` contiguous. A failed input is rewound past as well, so the
    -- next cell reuses its number: nothing can refer to it, a statement that
    -- throws having bound no `oN` and left `oo` where it was.
    User#"spectral end of cell" = () -> (
        lineNumber = lineNumber - 1;
        sendResult();
        if not completed then
            lineNumber = lineNumber - 1;
        forgetResult();
        new cellEnd
    );

    -- Announced once the hooks are installed and before the first cell can be
    -- read, so the shell knows the child is up and which M2 it got.
    emitFrame hashTable {"ready" => version#"VERSION"};
))()

-- Hand back the name the JSON transport borrowed. A freshly loaded package's
-- dictionary is inserted at the FRONT of dictionaryPath -- ahead of the user's
-- own -- so leaving it loaded reserves `json`, `toJSON`, `fromJSON` and the
-- formatting options. It does not merely shadow: `json = 3` fails outright with
-- "assignment to protected global variable 'toJSON'", naming a symbol the user
-- never typed. `dismiss` returns the names; the package itself stays loaded, so
-- the reference captured in the closure above keeps working.
dismiss JSON

-- Last, so that nothing above is echoed: from here the channel carries only what
-- the session produces.
echoOff()
topLevelMode = SpectralTopLevelMode

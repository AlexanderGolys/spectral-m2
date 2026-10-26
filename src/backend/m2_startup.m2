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

announceReady := (() -> (
    mode := global SpectralTopLevelMode;

    -- The rendered value and type annotation of the statement in flight. The
    -- hooks below only record them; the terminator sends them, because a
    -- statement that throws or ends in `;` never reaches Print, and the frame
    -- must leave exactly once per cell all the same.
    renderedValue := null;
    typeInfo := null;
    typeParts := null;
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
    -- An annotation is a sequence of pieces -- a type, a ring, a map's shape --
    -- each of which may be drawn over several rows: `QQ^2` raises its exponent.
    -- Its text joins the pieces' nets along their baselines, as M2 prints it.
    annotationPieces := value -> select(deepSplice sequence value, item -> item =!= null);

    Thing#{mode, BeforePrint} = value -> (
        lookupMethod := lookup({Standard, BeforePrint}, class value) ?? identity;
        lookupMethod value
    );
    -- What a value is drawn as, chosen by class. A frame's "value" is the
    -- shell's M2Value (src/backend.rs) in serde's spelling of an enum, a one-key
    -- table naming the variant:
    --
    --     net => text                   tex => {source, fallback}
    --     markup => {html, fallback,    hashtable => {{key, value}, ...}
    --         exampleWidth}
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

    -- A value that is one word with a role in code -- null, a Boolean, a
    -- string, a function -- is sent with that role, so the shell colours it as
    -- code colours the same word.
    asToken := (text, role) -> hashTable {
        symbol token => hashTable {symbol text => text, symbol role => role}
    };
    render Nothing := HashTable => value -> asToken("null", "null");
    render Boolean := HashTable => value -> asToken(toString value, "boolean");
    render String := HashTable => value -> asToken(value, "string");
    -- A function's TeX is its Net in a typewriter font, and nothing more. A
    -- named one is drawn by its name; an anonymous one's Net only says where it
    -- was written (`FunctionClosure[stdio:2:36-2:42]`), so it is drawn by its
    -- class instead.
    render Function := HashTable => value -> asToken(
        if hasAttribute(value, reverseDictionary) then toString value
        else toString class value,
        "function"
    );
    -- Every documentation node at once: DIV, TABLE, UL and the rest all descend
    -- from Hypertext. Its examples were run with the print width M2 gives every
    -- example, so their wide values were wrapped to it.
    exampleWidth := value Core#"private dictionary"#"ArgPrintWidthN";
    -- A link to another node points at the page M2 writes it to, named by
    -- `toFilename` of the node's key. Read back through the same encoding, each
    -- such link instead carries the key as an M2 string, which is what the
    -- shell asks `help` for when the link is clicked. Links elsewhere stay.
    toFilename := value Core#"private dictionary"#"toFilename";
    filenameCodes := hashTable apply(1..127, i -> (substring(toFilename ascii i, 1), ascii i));
    nodeKey := name -> (
        codes := characters substring(name, 1);
        key := {};
        i := 0;
        while i < #codes do (
            code := if codes#i == "_" then concatenate codes_{i..min(i + 2, #codes - 1)}
                else codes#i;
            if not filenameCodes#?code then return null;
            key = append(key, filenameCodes#code);
            i = i + #code);
        concatenate key);
    escapeAttribute := text -> replace(">", "&gt;", replace("<", "&lt;",
        replace("\"", "&quot;", replace("&", "&amp;", text))));
    linkedKeys := page -> (
        pattern := ///href="(?:[^"]*/)?(_[^"/]*)\.html"///;
        linked := "";
        while (match := regex(pattern, page)) =!= null do (
            key := nodeKey substring(match#1, page);
            linked = linked | substring(page, 0, match#0#0)
                | (if key === null then substring(match#0, page)
                   else "href=\"" | escapeAttribute format key | "\"");
            page = substring(page, match#0#0 + match#0#1));
        linked | page);
    render hypertextType := HashTable => value -> hashTable {
        symbol markup => hashTable {symbol html => linkedKeys html value, symbol fallback =>
                toString net value, symbol exampleWidth => exampleWidth}
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
    renderMutableHashTable := value -> (
        if lookup(net, class value) === lookup(net, MutableHashTable) then
            openMutable(value, table)
        else drawnWhole value
    );
    render MutableHashTable := HashTable => renderMutableHashTable;
    -- A type is a name in code, whatever kind of type it is -- Type,
    -- WrapperType, a class made by `new Type of` -- and so is a package. A ring,
    -- which is also a Type, draws itself, and so does a family of rings such as
    -- RR and CC, which stands for the ring at default precision.
    render Type := HashTable => value -> (
        if instance(value, Ring) or instance(value, RingFamily) then renderMutableHashTable value
        else asToken(toString value, "type")
    );
    render Package := HashTable => value -> asToken(toString value, "package");
    -- The words and operators the interpreter itself defines. An operator such as
    -- `^` or `SPACE` is coloured as code colours operators, a word such as `if`
    -- as a keyword.
    operators := unique join(
        value Core#"private dictionary"#"binaryOperators",
        value Core#"private dictionary"#"prefixOperators",
        value Core#"private dictionary"#"postfixOperators");
    render Keyword := HashTable => value -> asToken(toString value,
        if member(value, operators) then "operator" else "keyword");
    -- What is typed bare to steer the session, `exit` or `help`.
    render Command := HashTable => value -> asToken(toString value, "command");
    -- `Ext`, `HH` and the like are applied as functions, though they are
    -- hash tables that draw nothing of their own. One that carries a TeX
    -- spelling, `OO` as a calligraphic O, is set in it.
    render ScriptedFunctor := HashTable => value -> (
        if value.?texMath then drawnWhole value
        else asToken(toString value, "function")
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
    -- The names the shell completes, chosen as M2's own readline chooses them:
    -- every symbol in scope whose name begins with a letter or digit. Output
    -- history is left out, being recalled by number rather than typed. A
    -- dictionary is read again only once its size has changed, so a cell that
    -- binds nothing new sends nothing, and the shell keeps what it was sent.
    namesSent := new MutableHashTable;
    newNames := dictionaries -> unique flatten apply(
        select(dictionaries, dictionary -> dictionary =!= OutputDictionary),
        dictionary -> (
            if namesSent#?dictionary and namesSent#dictionary == #dictionary then {}
            else (
                namesSent#dictionary = #dictionary;
                select(keys dictionary, name -> match("^[[:alnum:]]", name))
            )
        )
    );
    -- `locals` are the top level's own local dictionaries, holding what a
    -- top-level `x := 1` binds; only the statement typed there can reach them.
    sendResult := locals -> emitFrame hashTable {
        "names" => newNames join(dictionaryPath, locals),
        "number" => lineNumber,
        "value" => renderedValue,
        "typeInfo" => typeInfo,
        "typeParts" => typeParts,
        "hasValue" => OutputDictionary#?("o" | toString lineNumber),
        "depth" => interpreterDepth,
        "error" => not completed
    };
    forgetResult := () -> (
        renderedValue = null;
        typeInfo = null;
        typeParts = null;
        completed = false;
        openingMutable = false;
    );
    recordTypeInfo := value -> (
        annotation := annotationPieces((lookup(AfterPrint, class value)) value ?? class value);
        typeInfo = toString horizontalJoin apply(annotation, net);
        -- Each piece as the shell renders a value, so the annotation is set from
        -- its structure rather than as the text of its nets.
        typeParts = apply(annotation, render);
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
    --
    -- Only the statement the user typed is recorded. `capture` -- which
    -- `installPackage` and `check` use to run examples -- reads its input through
    -- a top level of its own under the current mode, so these hooks also fire
    -- for every statement it evaluates. Recording those would mark a cell that
    -- later throws as completed, and swap its value for an example's; while the
    -- example results, captured from what the hooks print, would lose their
    -- values. `capture` forces `stopIfError`, which the shell's own top level
    -- never sets, so under it each hook is M2's stock one instead.
    atShellTopLevel := (key, hook) -> value -> (
        if stopIfError then (lookup({Standard, key}, class value) ?? identity) value
        else hook value
    );
    Thing#{mode, Print} = atShellTopLevel(Print, value -> (
        renderedValue = render value;
    ));
    Thing#{mode, NoPrint} = atShellTopLevel(NoPrint, identity);
    -- Deliberately AfterPrint in both branches: stock M2 reads AfterNoPrint under
    -- a `;`, which only some classes define, so the type would appear or vanish
    -- with the punctuation. Reading AfterPrint always gives every class its type.
    -- These run last for a statement that completed, which is what makes reaching
    -- them the definition of having completed.
    Thing#{mode, AfterPrint} = atShellTopLevel(AfterPrint, recordTypeInfo);
    Thing#{mode, AfterNoPrint} = atShellTopLevel(AfterNoPrint, recordTypeInfo);

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
    -- Keep interruption from leaving a partial frame or half-reset cell state.
    User#"spectral end of cell" = locals -> shield (
        lineNumber = lineNumber - 1;
        sendResult locals;
        if not completed then
            lineNumber = lineNumber - 1;
        forgetResult();
        new cellEnd
    );

    -- Announced once the hooks are installed and before the first cell can be
    -- read, so the shell knows the child is up, which M2 it got, and every name
    -- the session starts with. Called below, once the JSON names are given back.
    () -> emitFrame hashTable {"ready" => version#"VERSION", "names" => newNames dictionaryPath}
))()

-- Hand back the name the JSON transport borrowed. A freshly loaded package's
-- dictionary is inserted at the FRONT of dictionaryPath -- ahead of the user's
-- own -- so leaving it loaded reserves `json`, `toJSON`, `fromJSON` and the
-- formatting options. It does not merely shadow: `json = 3` fails outright with
-- "assignment to protected global variable 'toJSON'", naming a symbol the user
-- never typed. `dismiss` returns the names; the package itself stays loaded, so
-- the reference captured in the closure above keeps working.
dismiss JSON
announceReady()

-- Last, so that nothing above is echoed: from here the channel carries only what
-- the session produces.
echoOff()
topLevelMode = SpectralTopLevelMode

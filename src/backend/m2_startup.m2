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
    -- The rendered value and type annotation of the statement in flight, filled
    -- by the hooks below and read once by the frame that closes the input.
    output := new MutableHashTable from {
        symbol value => null,
        symbol typeInfo => null
    };
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
    -- Set by the terminator and cleared by the hook pass that immediately follows
    -- it, so the shell's own statement never reports itself as a result. It spans
    -- one statement boundary and is read nowhere else.
    ownStatement := false;
    renderAfterPrint := value -> "" | concatenate apply(
        select(deepSplice sequence value, item -> item =!= null),
        item -> toString net item
    );

    Thing#{mode, BeforePrint} = value -> (
        lookupMethod := lookup({Standard, BeforePrint}, class value);
        if lookupMethod === null then
            value
        else lookupMethod value
    );
    -- Recording rather than printing keeps the channel free of everything but
    -- genuinely printed text. A `;` still suppresses the value: NoPrint
    -- deliberately skips `net value`, which is the render the punctuation exists
    -- to avoid on large objects. The type is read either way, so the shell can
    -- say what a suppressed result is without paying to render it.
    renderAtom := value -> (
        netValue := "" | toString net value;
        if instance(value, FunctionClosure) or instance(value, CompiledFunctionClosure) then
            hashTable {"net" => netValue}
        else try hashTable {
                "tex" => hashTable {
                    "source" => tex value,
                    "fallback" => netValue
                }
            } else hashTable {"net" => netValue}
    );
    renderValue := value -> (
        if instance(value, hypertextType) then hashTable {
            "markup" => hashTable {
                "html" => html value,
                "fallback" => "" | toString net value
            }
        }
        else if class value === HashTable then hashTable {
            "hashtable" => apply(pairs value, pair -> apply(toList pair, renderAtom))
        }
        else if class value === VerticalList then hashTable {
            "verticallist" => apply(toList value, renderAtom)
        }
        else renderAtom value
    );
    recordValue := value -> (
        output.value = renderValue value;
    );
    recordInexactValue := value -> (
        savedPrecision := printingPrecision;
        savedAccuracy := printingAccuracy;
        printingPrecision = 0;
        printingAccuracy = -1;
        recordValue value;
        printingPrecision = savedPrecision;
        printingAccuracy = savedAccuracy;
    );

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
        "value" => output.value,
        "typeInfo" => output.typeInfo,
        -- Read from M2's own record rather than tracked separately: `oN` exists
        -- exactly when the result was bound, which is what the shell is asking.
        -- It is not implied by `value`, a `;`-suppressed result being bound
        -- without ever being rendered.
        "hasValue" => OutputDictionary#?("o" | toString lineNumber),
        -- Nested interpreters are how M2 says it is sitting in the debugger;
        -- the shell paints itself differently when it is.
        "depth" => interpreterDepth,
        "error" => not completed
    };
    forgetResult := () -> (
        output.value = null;
        output.typeInfo = null;
        completed = false;
    );
    recordTypeInfo := value -> (
        -- The terminator's own pass through here, cleared as it is recognised.
        if ownStatement then (
            ownStatement = false;
            return;
        );
        lookupMethod := lookup(AfterPrint, class value);
        typeInfo := lookupMethod value;
        if typeInfo === null then
            typeInfo = class value;
        output.typeInfo = renderAfterPrint typeInfo;
        completed = true;
    );

    -- Nothing deliberately has no Print override: an unterminated statement
    -- reports its result whatever it is, and `net null` is already "null". A `;`
    -- still silences it, since NoPrint stays identity for every class.
    Thing#{mode, Print} = recordValue;
    Thing#{mode, NoPrint} = identity;
    InexactNumber#{mode, Print} = recordInexactValue;
    -- Deliberately AfterPrint in both branches: stock M2 reads AfterNoPrint under
    -- a `;`, which only some classes define, so the type would appear or vanish
    -- with the punctuation. Reading AfterPrint always gives every class its type.
    -- These run last for a statement that completed, which is what makes reaching
    -- them the definition of having completed.
    Thing#{mode, AfterPrint} = recordTypeInfo;
    Thing#{mode, AfterNoPrint} = recordTypeInfo;
    protect mode;

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
        ownStatement = true;
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

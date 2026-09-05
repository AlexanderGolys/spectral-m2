-- spectral-m2 startup script
-- Reads the listen port from the script argument, opens a listener, and serves
-- JSON-RPC 2.0 requests over a loopback TCP socket.
--
-- The Rust crate's TCP transport opens one connection per request, so the
-- connection is the message boundary: M2 reads one JSON value, dispatches it via
-- the JSONRPC package, writes one response, and closes the connection.

needsPackage "JSONRPC"
needsPackage "JSON"

-- The TUI passes the port to listen on as the script's sole argument
-- (scriptCommandLine#0 is this script's own path, #1 the port). This avoids a
-- shared rendezvous file the two processes could disagree on (e.g. under $TMPDIR).
serverHost = "127.0.0.1"
port = value scriptCommandLine#1

rpcServer = new JSONRPCServer;

(() -> (
    mode := global SpectralTopLevelMode;
    -- Rendered value, type annotation, and whether an oN was bound. The last is
    -- not implied by the first: a `;`-suppressed result binds oN without ever
    -- being rendered. A MutableHashTable, not a MutableList: the hooks below run
    -- on the task thread that `evaluate` schedules, and M2 documents mutable hash
    -- tables as thread safe while changes inside mutable lists are not.
    output := new MutableHashTable from {
        symbol value => null,
        symbol typeInfo => null,
        symbol hasValue => false
    };
    -- The input number of the evaluation in flight, or null when none is: the
    -- shell's iN, which is also the name the result must be bound under.
    inputNumber := null;
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
    -- Recording rather than printing keeps the captured transcript free of
    -- everything but genuinely printed text. A `;` still suppresses the value:
    -- NoPrint deliberately skips `net value`, which is the render the punctuation
    -- exists to avoid on large objects. The type is read either way, so the shell
    -- can say what a suppressed result is without paying to render it.
    recordValue := value -> output.value = "" | toString net value;
    recordInexactValue := value -> (
        savedPrecision := printingPrecision;
        savedAccuracy := printingAccuracy;
        printingPrecision = 0;
        printingAccuracy = -1;
        recordValue value;
        printingPrecision = savedPrecision;
        printingAccuracy = savedAccuracy;
    );
    recordTypeInfo := value -> (
        lookupMethod := lookup(AfterPrint, class value);
        typeInfo := lookupMethod value;
        if typeInfo === null then
            typeInfo = class value;
        output.typeInfo = renderAfterPrint typeInfo;
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
    Thing#{mode, AfterPrint} = recordTypeInfo;
    Thing#{mode, AfterNoPrint} = recordTypeInfo;
    protect mode;

    -- Bind oN and rotate oo/ooo/oooo, mirroring Core.m2's Thing#AfterEval. Ours
    -- replaces it for two reasons. First, `capture` restarts its inner session at
    -- lineNumber 1, so Core's version names every result o1 and overwrites it --
    -- the shell supplies the true input number instead. Second, the gate on
    -- inputNumber keeps any capture the shell runs on its own behalf -- the user
    -- init file, say -- from rotating oo or claiming a number. AfterEval is not
    -- keyed by topLevelMode, so this is a global
    -- replacement -- deliberate, since this process serves only the shell.
    -- TODO the output bookkeeping should be inviolable -- `protect symbol oo`
    -- must be refused rather than allowed to disable it. Until then this matches
    -- Core's unguarded assignment, which a user can still break by protecting oo.
    Thing#AfterEval = value -> (
        if inputNumber =!= null then (
            if ooo =!= null then
                global oooo <- ooo;
            if oo =!= null then
                global ooo <- oo;
            if value =!= null then (
                global oo <- value;
                output.hasValue = true;
                (getGlobalSymbol(OutputDictionary,
                    concatenate("o", toString inputNumber))) <- value;
            );
        );
        value
    );

    -- `number` is the shell's iN for this cell, or null for code the session runs
    -- on its own behalf (the user init file), whose results bind no oN.
    registerMethod(rpcServer, "evaluate", {"code", "number"}, (code, number) -> (
        output.value = null;
        output.typeInfo = null;
        output.hasValue = false;
        inputNumber = number;
        previousTopLevelMode := topLevelMode;
        topLevelMode = mode;
        -- Same hazard `check` documents: JSONRPC traps method errors, and that
        -- outer trap swallows the diagnostic before `capture` can write it into
        -- the transcript -- an errored evaluation came back with the echoed input
        -- and nothing else. Running the capture in its own task keeps the message.
        (err, out) := taskResult schedule(() -> capture(code, UserMode => true));
        topLevelMode = previousTopLevelMode;
        inputNumber = null;
        hashTable {
            "transcript" => out,
            "value" => output.value,
            "typeInfo" => output.typeInfo,
            "hasValue" => output.hasValue,
            -- Nested interpreters are how M2 says it is sitting in the debugger;
            -- the shell paints itself differently when it is.
            "depth" => interpreterDepth,
            "error" => err
        }
    ));
))();

registerMethod(rpcServer, "version", {}, () -> version#"VERSION");

-- Serve requests, having first handed the user back the names our transport
-- borrowed. Neither JSON nor JSONRPC is in M2's prelude, and a freshly loaded
-- package's dictionary is inserted at the FRONT of dictionaryPath -- ahead of
-- the user's own -- so leaving them loaded reserves thirteen unqualified names
-- (json, toJSON, fromJSON, handleRequest, registerMethod, setLogger,
-- makeRequest, Indent, IndentLevel, NameSeparator, ValueSeparator,
-- JSONRPCServer, JSONRPCError). It does not merely shadow: `json = 3` fails
-- outright with "assignment to protected global variable 'toJSON'", naming a
-- symbol the user never typed. `dismiss` returns the names; a user who wants
-- them can `needsPackage "JSON"` themselves, which re-adds the dictionary to the
-- path at no cost, since the package itself stays loaded.
--
-- This is deliberately ONE statement. M2 binds names as each statement is
-- parsed, so the two references below resolve while the dictionaries are still
-- on the path and keep working after the dismiss. Split them out and they do not
-- error -- they silently become their own unbound Symbols, failing later, at the
-- first request.
(() -> (
    -- The connection delivers a request in however many chunks TCP chooses, so
    -- `payload` is routinely a partial JSON document and "does it parse yet" is
    -- the read loop's exit condition. `??` guards its LEFT operand only -- errors
    -- on the right propagate -- so the parse must sit there to yield null rather
    -- than raise on an incomplete read.
    parseRequest := payload -> fromJSON payload ?? null;
    serveRequest := payload -> handleRequest(rpcServer, payload);
    dismiss JSON;
    dismiss JSONRPC;

    listener := openListener concatenate("$", serverHost, ":", toString port);
    while true do (
        conn := openInOut listener;
        payload := "";
        request := null;
        -- `read` yields the empty string at end of file. Without this guard a peer
        -- that hangs up before it finished sending -- the Rust side reaching its
        -- read timeout, say -- spins here forever, pinning a core and wedging the
        -- listener against every later connection as well.
        while request === null do (
            chunk := read conn;
            if length chunk === 0 then break;
            payload = payload | chunk;
            request = parseRequest payload;
        );
        if request =!= null then (conn << serveRequest payload << flush);
        close conn;
    )
))()

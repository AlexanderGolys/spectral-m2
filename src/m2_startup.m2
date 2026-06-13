-- spectral-m2 startup script
-- Reads the port from /tmp/spectral-m2_port (written by Rust), opens a listener,
-- and serves LENGTH-PREFIXED request/response frames over the socket.
--
-- Framing is length-prefixed so a payload (user code, or M2 output) may contain
-- ANY bytes -- newlines, multi-line strings/comments, even the marker strings --
-- without breaking the frame. A control-marker line names the verb; the next line
-- is a decimal byte count; then exactly that many payload bytes follow. (M2
-- strings, `length`, and `substring` are byte-based, so the count is in bytes.)
--
--   M2 -> Rust once:   <<<M2_READY>>>\n
--   Evaluate:
--     Rust -> M2:  <<<M2_CODE>>>\n<n>\n<n bytes of code>
--     M2 -> Rust:  <<<M2_RESULT>>>\n<ok|error>\n<n>\n<n bytes of output>
--   Completeness check (NON-EXECUTING -- only parses, never evaluates):
--     Rust -> M2:  <<<M2_CHECK>>>\n<n>\n<n bytes of code>
--     M2 -> Rust:  <<<M2_VERDICT>>>\n<COMPLETE|INCOMPLETE|ERROR>\n<n>\n<n bytes of message>
--
-- These framing constants are mirrored in src/m2_backend.rs and must stay in sync.
-- Globals use distinctive names: short lowercase names risk colliding with M2
-- keywords (e.g. `step`), and binding a keyword is undefined behaviour.

portFile = "/tmp/spectral-m2_port"
f = openIn portFile
portStr = read f
close f
port = value portStr
listener = openListener concatenate("$127.0.0.1:", toString port)
conn = openInOut listener
conn << "<<<M2_READY>>>" << endl << flush

-- Does the parse tree contain the bare end-of-file pseudo-token, i.e. did parsing
-- reach the end of input? A string LITERAL "-*end of file*-" (regular or raw)
-- parses to a *quoted* token, distinct from this bare one, so the test is
-- contamination-proof.
hasEofToken = node -> (
    if instance(node, BasicList) then (
        if #node == 2 and node#0 === "Token" and node#1 === "-*end of file*-"
            then return true;
        any(toList node, hasEofToken)
    ) else false
);

-- Classify a buffer (pure string op, never evaluates user code). Returns
-- (verdict, message). `parse` mutates its argument, so always parse a fresh copy.
parseCheck = src -> (
    result := try parse(src | "") else null;
    if result =!= null then (
        if hasEofToken result then ("INCOMPLETE", "") else ("COMPLETE", "")
    ) else (
        -- parse threw: re-run under capture to read the PRINTED diagnostic (the
        -- caught error is only a generic wrapper; the detail is in the print).
        (err, out) := capture("parse(" | format src | ")", UserMode => false);
        errLines := select(lines out, l -> match(///error:///, l) and not match(///backtrace///, l));
        reachedEof := any(errLines, l ->
            match(///at '-\*end of file\*-'///, l)   -- ran out of input
            or match(///EOF in string///, l)          -- unterminated string literal
            or match(///to match this///, l));         -- unclosed if/for/while keyword
        if reachedEof then ("INCOMPLETE", "")
        else ("ERROR", if #errLines > 0 then first errLines else "syntax error")
    )
);

-- Write a length-prefixed reply: marker line, header line, byte count, payload.
sendReply = (marker, header, payload) -> (
    conn << marker << endl << header << endl
         << toString length payload << endl << payload << flush;
);

frameBuf = "";
frameState = "marker";   -- marker -> length -> payload -> marker -> ...
frameMode = "none";      -- "eval" or "check" once a start marker is seen
frameNeed = 0;

-- Advance over frameBuf by one step; returns true if progress was made (call
-- again), false if more bytes are required from the socket.
pumpFrame = () -> (
    if frameState === "payload" then (
        if length frameBuf < frameNeed then false
        else (
            payload := substring(0, frameNeed, frameBuf);
            frameBuf = substring(frameNeed, frameBuf);
            if frameMode === "eval" then (
                (err, output) := capture(payload, UserMode => true);
                sendReply("<<<M2_RESULT>>>", if err then "error" else "ok", output);
            ) else (
                (verdict, msg) := parseCheck payload;
                sendReply("<<<M2_VERDICT>>>", verdict, msg);
            );
            frameState = "marker"; frameMode = "none";
            true
        )
    ) else (
        nl := regex("\n", frameBuf);
        if nl === null then false
        else (
            idx := (nl#0)#0;
            line := substring(0, idx, frameBuf);
            frameBuf = substring(idx + 1, frameBuf);
            if frameState === "marker" then (
                if line === "<<<M2_CODE>>>" then (frameMode = "eval"; frameState = "length")
                else if line === "<<<M2_CHECK>>>" then (frameMode = "check"; frameState = "length");
            ) else (
                frameNeed = value line;
                frameState = "payload";
            );
            true
        )
    )
);

while true do (
    chunk := read conn;
    if length chunk === 0 then break;  -- connection closed
    frameBuf = frameBuf | chunk;
    while pumpFrame() do null;
);

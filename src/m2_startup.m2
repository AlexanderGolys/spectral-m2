-- spectral-m2 startup script
-- Reads the listen port from the script argument, opens a listener, and serves
-- LENGTH-PREFIXED JSON request/response frames over the socket.
--
-- Framing is length-prefixed so a payload (a JSON document) may contain ANY bytes
-- -- newlines, quotes, even the marker string -- without breaking the frame. Each
-- frame is a marker line, a decimal byte count line, then exactly that many payload
-- bytes. (M2 strings, `length`, and `substring` are byte-based, so the count is in
-- bytes.) The payloads are JSON, escaped/parsed by the `JSON` package:
--
--   M2 -> Rust once:   <<<M2_READY>>>\n
--   Per request:
--     Rust -> M2:  <<<M2_MSG>>>\n<n>\n<n bytes: {"method": "evaluate"|"check", "code": ...}>
--     M2 -> Rust:  <<<M2_MSG>>>\n<n>\n<n bytes: JSON result>
--       evaluate -> {"output": <string>, "error": <bool>}
--       check    -> {"verdict": "COMPLETE"|"INCOMPLETE"|"ERROR", "message": <string>}
--
-- The marker string, method names, and result field names are mirrored in
-- src/m2_backend.rs and must stay in sync. NB: we use the `JSON` package directly
-- (manual method dispatch below) rather than the `JSONRPC` package, because the
-- latter's `handleRequest` wraps each call in error-trapping that breaks
-- `parseCheck`'s `try parse` completeness detection (an incomplete buffer is then
-- misreported as a syntax error). Globals use distinctive names: short lowercase
-- names risk colliding with M2 keywords (e.g. `step`), and binding a keyword is
-- undefined behaviour.

needsPackage "JSON"

-- The TUI passes the port to listen on as the script's sole argument
-- (scriptCommandLine#0 is this script's own path, #1 the port). This avoids a
-- shared rendezvous file the two processes could disagree on (e.g. under $TMPDIR).
serverHost = "127.0.0.1"
port = value scriptCommandLine#1

-- openListener's "$host:port" form opens a server socket bound to the loopback host.
listener = openListener concatenate("$", serverHost, ":", toString port)
conn = openInOut listener


-- Frame marker, mirrored in src/m2_backend.rs -- keep in sync.
M2MSG := "<<<M2_MSG>>>";
M2READY := "<<<M2_READY>>>";
EOF := "-*end of file*-";

conn << M2READY << endl << flush

-- Does the parse tree contain the bare end-of-file pseudo-token, i.e. did parsing
-- reach the end of input? A string LITERAL "-*end of file*-" (regular or raw)
-- parses to a *quoted* token, distinct from this bare one, so the test is
-- contamination-proof.
hasEofToken = node -> (
    if instance(node, BasicList) then (
        if #node == 2 and node#0 === "Token" and node#1 === EOF then
            return true;
        any(toList node, hasEofToken)
    ) else
        false
);

-- Classify a buffer (pure string op, never evaluates user code). Returns
-- (verdict, message). `parse` mutates its argument, so always parse a fresh copy.
parseCheck = src -> (
    result := try parse(src | "");
    if result =!= null then (
        if hasEofToken result then (
            "INCOMPLETE", ""
        ) else (
            "COMPLETE", ""
        )
    ) else (
        -- parse threw: re-run under capture to read the PRINTED diagnostic (the
        -- caught error is only a generic wrapper; the detail is in the print).
        (err, out) := capture("parse(" | format src | ")", UserMode => false);
        errLines := select(lines out, l -> match(///error:///, l) and not match(///backtrace///, l));
        reachedEof := any(errLines, l ->
                match(///at '-\*end of file\*-'///, l)   -- ran out of input
            or match(///EOF in string///, l)          -- unterminated string literal
            or match(///to match this///, l)
        );
        -- unclosed if/for/while keyword
        if reachedEof then (
            "INCOMPLETE", ""
        ) else (
            "ERROR", if #errLines > 0 then first errLines else "syntax error"
        )
    )
);

-- Dispatch one JSON request string to its handler, returning a JSON response
-- string. `evaluate` executes the code (capturing its session output); `check`
-- only classifies completeness. Both reply with a single JSON object.
handleMessage = payload -> (
    req := fromJSON payload;
    code := req#"code";
    method := req#"method";
    if method === "evaluate" then (
        (err, out) := capture(code, UserMode => true);
        toJSON hashTable {"output" => out, "error" => err}
    ) else if method === "check" then (
        (verdict, msg) := parseCheck code;
        toJSON hashTable {"verdict" => verdict, "message" => msg}
    ) else
        toJSON hashTable {"output" => "unknown method: " | method, "error" => true}
);

-- Write a length-prefixed frame: marker line, byte count, payload.
sendMessage = payload -> (
    conn << M2MSG << endl << toString length payload << endl << payload << flush;
);

frameBuf = "";
frameState = "marker";
-- marker -> length -> payload -> marker -> ...
frameNeed = 0;

-- Advance over frameBuf by one step; returns true if progress was made (call
-- again), false if more bytes are required from the socket.
pumpFrame = () -> (
    if frameState === "payload" then (
        if length frameBuf < frameNeed then (
            false
        ) else (
            payload := substring(0, frameNeed, frameBuf);
            frameBuf = substring(frameNeed, frameBuf);
            sendMessage(handleMessage payload);
            frameState = "marker";
            true
        )
    ) else (
        nl := regex("\n", frameBuf);
        if nl === null then (
            false
        ) else (
            idx := (nl#0)#0;
            line := substring(0, idx, frameBuf);
            frameBuf = substring(idx + 1, frameBuf);
            if frameState === "marker" then (
                if line === M2MSG then frameState = "length";
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
    if length chunk === 0 then
        break;
    -- connection closed
    frameBuf = frameBuf | chunk;
    while pumpFrame() do null;
)


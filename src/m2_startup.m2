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

COMPLETE := "COMPLETE";
INCOMPLETE := "INCOMPLETE";
ERROR := "ERROR";

protect symbol COMPLETE;
protect symbol INCOMPLETE;
protect symbol ERROR;




-- Does the parse tree contain the bare end-of-file pseudo-token, i.e. did parsing
-- reach the end of input? A string LITERAL "-*end of file*-" (regular or raw)
-- parses to a *quoted* token, distinct from this bare one, so the test is
-- contamination-proof.

isEOFToken = node -> instance(node, BasicList) and
                     #node == 2 and
                     instance(node#0, String) and
                     node#0 == "Token" and
                     instance(node#1, String) and
                     node#1 == "-*end of file*-";

hasEofToken = node -> isEOFToken node or
                      instance(node, BasicList) and any(node, hasEofToken);

outputErrorIncomplete = out -> (
    errs := select(lines out, l -> match(": error:", l)) / (l -> last separate("error: ", l));
    if #errs == 1 then
        match(///syntax error at '-\*end of file\*-'///, errs#0) or
        match(///EOF in string///, errs#0)
    else if #errs == 2 then
        match(///expected///, errs#0) and match(///to match this///, errs#1) or
        match(///EOF in string///, errs#0) and match(///unmatched -\*error token\*-///, errs#1)

    else
        false
);

errorMsg := out -> (
    try
        separate("error: ", select(lines out, l -> match(": error:", l))#0)#1
    else
        "unknown error"
);


-- Classify a buffer (pure string op, never evaluates user code). Returns
-- (verdict, message). `parse` mutates its argument, so always parse a fresh copy.
parseCheck = src -> (
    (err, out) := capture("parse(" | format src | ")", UserMode => false);
    if err then
        if outputErrorIncomplete out then
            (INCOMPLETE,)
        else
            (ERROR, errorMsg out)
    else if hasEofToken value("parse(" | format src | ")") then
        (INCOMPLETE,)
    else
        (COMPLETE,)
);



rpcServer = new JSONRPCServer;

registerMethod(rpcServer, "evaluate", {"code"}, code -> (
        (err, out) := capture(code, UserMode => true);
        hashTable {"output" => out, "error" => err}
    ));

registerMethod(rpcServer, "check", {"code"}, code -> (
        -- JSONRPC traps method errors. Run the parser capture in its own task so
        -- it retains the original syntax diagnostic before that outer trap.
        reply := taskResult schedule(() -> parseCheck code);
        verdict := reply#0;
        msg := reply#1 ?? "";
        hashTable {"verdict" => verdict, "message" => msg}
    ));

registerMethod(rpcServer, "version", {}, () -> version#"VERSION");

LISTENER = openListener concatenate("$", serverHost, ":", toString port)

while true do (
    conn := openInOut LISTENER;
    payload := "";
    request := null;
    while request === null do (
        payload = payload | read conn;
        request = try fromJSON payload else null;
    );
    response := handleRequest(rpcServer, payload);
    conn << response << flush;
    close conn;
)

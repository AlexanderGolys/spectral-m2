-- pretty-shell startup script
-- Reads port from /tmp/m2sh_port (written by Rust), opens listener, enters eval loop.
--
-- Protocol:
--   Rust writes port to /tmp/m2sh_port
--   M2 -> REPL (socket): <<<M2_READY>>>
--   REPL -> M2 (socket): <<<M2_CODE>>>\n<code>\n<<<M2_END>>>\n
--   M2 -> REPL (socket): <<<M2_RESULT>>>\n<error|ok>\n<output>\n<<<M2_END>>>\n

portFile = "/tmp/m2sh_port"
f = openIn portFile
portStr = read f
close f
port = value portStr
listener = openListener concatenate("$127.0.0.1:", toString port)
conn = openInOut listener
conn << "<<<M2_READY>>>" << endl << flush

while true do (
    line = read conn;
    if line === "<<<M2_CODE>>>" then (
        code = "";
        while true do (
            line = read conn;
            if line === "<<<M2_END>>>" then break;
            if length code > 0 then code = code | newline;
            code = code | line;
        );
        (err, output) := capture(code, UserMode => true);
        conn << "<<<M2_RESULT>>>" << endl;
        if err then conn << "error" << endl
        else conn << "ok" << endl;
        conn << output << endl;
        conn << "<<<M2_END>>>" << endl << flush;
    )
)
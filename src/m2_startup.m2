-- spectral-m2 startup script
-- Reads port from /tmp/spectral-m2_port (written by Rust), opens listener, enters eval loop.
--
-- Protocol:
--   Rust writes port to /tmp/spectral-m2_port
--   M2 -> REPL (socket): <<<M2_READY>>>
--   REPL -> M2 (socket): <<<M2_CODE>>>\n<code>\n<<<M2_END>>>\n
--   M2 -> REPL (socket): <<<M2_RESULT>>>\n<error|ok>\n<output>\n<<<M2_END>>>\n
--
-- `read conn` returns whatever bytes are currently available (often a whole
-- framed message at once, not a single line), so we buffer the incoming bytes
-- and split them into complete lines ourselves before matching the markers.

portFile = "/tmp/spectral-m2_port"
f = openIn portFile
portStr = read f
close f
port = value portStr
listener = openListener concatenate("$127.0.0.1:", toString port)
conn = openInOut listener
conn << "<<<M2_READY>>>" << endl << flush

inBuf = "";
collecting = false;
srcCode = "";

while true do (
    chunk := read conn;
    if length chunk === 0 then break;  -- connection closed
    inBuf = inBuf | chunk;
    -- separate yields one trailing element for the (possibly partial) final
    -- line; keep it buffered and process every complete line before it.
    parts := separate("\n", inBuf);
    n := #parts;
    inBuf = parts#(n - 1);
    for i from 0 to n - 2 do (
        line := parts#i;
        if not collecting then (
            if line === "<<<M2_CODE>>>" then (collecting = true; srcCode = "";);
        ) else (
            if line === "<<<M2_END>>>" then (
                (err, output) := capture(srcCode, UserMode => true);
                conn << "<<<M2_RESULT>>>" << endl;
                if err then conn << "error" << endl else conn << "ok" << endl;
                conn << output << endl;
                conn << "<<<M2_END>>>" << endl << flush;
                collecting = false;
            ) else (
                if length srcCode > 0 then srcCode = srcCode | newline;
                srcCode = srcCode | line;
            );
        );
    );
);

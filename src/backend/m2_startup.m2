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

-- Loaded by the shell ahead of everything else, so a session's init file can
-- already draw its own classes; named here as what the hooks are built on.
needsPackage "Spectral"

announceReady := (() -> (
	mode := global SpectralTopLevelMode;

	-- Frames are encoded by `encodeFrame` rather than by the JSON package, whose
    -- encoder reaches `toString` through the session's method tables: a user's
    -- `toString ZZ := x -> "a"` turned every number in a frame into `a`. These
    -- three are taken once, before any user code runs, and a later redefinition
    -- replaces a table entry without touching the function captured here.
	integerText := lookup(toString, ZZ);
	symbolText := lookup(toString, Symbol);
	stringLiteral := lookup(format, String);

	-- The stock debugger hook prints `code current`. Source submitted through
    -- stdin has no readable file behind it; failure to display it must not
    -- unwind debugger entry and strand its interpreter. Keep the stock hook's
    -- inDebugger push/pop behavior, treating its display as best effort.
	debuggerHookSymbol := Core#"private dictionary"#"debuggerHook";
	originalDebuggerHook := value debuggerHookSymbol;
	shield (
		-- Replacing this hook is intentional. Suppress the function-release
        -- warning for this symbol only, then restore the normal release hook.
		releaseHook := Function.GlobalReleaseHook;
		Function.GlobalReleaseHook = (name, function) -> (
			if name =!= debuggerHookSymbol then
				releaseHook(name, function)
		);
		debuggerHookSymbol <- (entering -> try originalDebuggerHook entering);
		Function.GlobalReleaseHook = releaseHook;
	);

	-- The rendered value and type annotation of the statement in flight. The
    -- hooks below only record them; the terminator sends them, because a
    -- statement that throws or ends in `;` never reaches Print, and the frame
    -- must leave exactly once per cell all the same.
	renderedValue := null;
	previews := {};
	typeInfo := null;
	typeParts := null;

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
	-- What a value is drawn as is the Spectral package's business: `specNet`,
	-- which a session may install for a class of its own. The formatting hooks are
	-- taken once, before any user code runs, so rebinding a name later cannot
	-- change what is called here. `drawnAs` is `specNet` held to giving a
	-- SpecNet; `forgetDrawing` clears what a drawing that threw part-way may have
	-- left behind.
	spectral := Spectral#"private dictionary";
	drawnAs := value spectral#"drawnAs";
	typedAs := value spectral#"typedAs";
	forgetDrawing := value spectral#"forgetDrawing";
	-- Whether drawing the statement's value failed. M2 reports an error raised
	-- while printing and carries on into AfterPrint, so reaching AfterPrint alone
	-- does not show that the statement came through whole.
	drawingFailed := false;
	-- A class in a frame -- the role of a word, the kind of a list -- is sent as
	-- its ancestors, nearest first, for the shell to read by the first it knows.
	typeName := lookup(toString, Type);

	-- One frame closes one input. Marked with a control character no diagnostic
    -- and no prompt can begin with, so the shell can pick frames out of the
    -- stream by inspecting the first byte alone; printed text that happens to
    -- start with it is the user's own doing and reads back as a frame, which is
    -- the same bargain any in-band marker makes.
	frameMark := ascii 29;
	encodeFrame := null;
	encodeFrame = value -> (
		if value === null then
			"null"
		else if instance(value, Boolean) then if value then
			"true"
		else "false" else if instance(value, ZZ) then
			integerText value
		else if instance(value, String) then
			stringLiteral value
		else if instance(value, VisibleList) then
			"[" | demark(", ", apply(value, encodeFrame)) | "]"
		else if instance(value, Type) then
			encodeFrame apply(ancestors value, typeName)
		else if instance(value, HashTable) then
			"{" | demark(", ", apply(keys value, key ->
					stringLiteral(if instance(key, Symbol) then
								symbolText key
							else key)
					| ": " | encodeFrame value#key)) | "}"
		else error "a frame holds only null, booleans, integers, strings, lists, classes and tables"
	);
	emitFrame := payload -> (
		stdio << frameMark << encodeFrame payload << endl << flush;
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
			if namesSent#?dictionary and namesSent#dictionary
				== #dictionary then {} else (
					namesSent#dictionary = #dictionary;
					select(keys dictionary, name -> match("^[[:alnum:]]", name))
				)
		)
	);
	-- `locals` are the top level's own local dictionaries, holding what a
    -- top-level `x := 1` binds; only the statement typed there can reach them.
	sendResult := (submission, locals) -> emitFrame hashTable {
		"submission" => submission,
		"names" => newNames join(dictionaryPath, locals),
		"number" => lineNumber,
		"value" => renderedValue,
		"previews" => previews,
		"typeInfo" => typeInfo,
		"typeParts" => typeParts,
		"hasValue" => OutputDictionary#?("o" | integerText lineNumber),
		"depth" => interpreterDepth,
		"error" => not completed
	};
	forgetResult := () -> (
		renderedValue = null;
		previews = {};
		typeInfo = null;
		typeParts = null;
		completed = false;
		drawingFailed = false;
		forgetDrawing();
	);
	recordTypeInfo := value -> (
		completed = false;
		typeParts = {typedAs value};
		-- Keep stock text for the explicitly requested original-output view.
		annotation := annotationPieces((lookup(AfterPrint, class value)) value ?? class value);
		typeInfo = toString horizontalJoin apply(annotation, net);
		completed = not drawingFailed;
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
		if stopIfError then
			(lookup({Standard, key}, class value) ?? identity) value
		else hook value
	);
	Thing#{mode, Print} = atShellTopLevel(Print, value -> (
        completed = false;
		drawingFailed = true;
		renderedValue = drawnAs value;
		previews = select(apply({
            ("net", x -> toString net x),
            ("string", toString),
            ("tex", tex),
            ("external", toExternalString)
        }, (name, formatter) -> (
            text := try formatter value;
            if instance(text, String) then hashTable {"mode" => name, "text" => text}
        )), preview -> preview =!= null);
		drawingFailed = false;
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
    -- `submission` is the shell's own count of what it has sent, echoed back so a
    -- frame that arrives for an earlier submission is recognised and dropped
    -- instead of being filed under the cell now waiting.
    -- Keep interruption from leaving a partial frame or half-reset cell state.
	User#"spectral end of cell" = (submission, locals) -> shield (
		lineNumber -= 1;
		sendResult(submission, locals);
		if not completed then
			lineNumber -= 1;
		forgetResult();
		new cellEnd
	);

	-- Announced once the hooks are installed and before the first cell can be
    -- read, so the shell knows the child is up, which M2 it got, and every name
    -- the session starts with. Called below.
	() -> emitFrame hashTable {
		"ready" => version#"VERSION",
		"names" => newNames dictionaryPath
	}
))()

announceReady()

-- Last, so that nothing above is echoed: from here the channel carries only what
-- the session produces.
echoOff()
topLevelMode = SpectralTopLevelMode

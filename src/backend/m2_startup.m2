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

-- Installs the shell's mode and announces it; evaluates to the mode, which the
-- top level switches to last of all.
spectralShell := (() -> (
	-- The mode M2 dispatches on: a symbol no session can name, whose hooks are
    -- installed on Thing alone. The protocol they keep is the shell's own, not
    -- something to override; a session changes how values are drawn through the
    -- Spectral package's `rod` and `specType`, which these hooks only call.
	shell := local SpectralShell;
	standardStage := (stage, value) -> (lookup({Standard, stage}, class value)
			?? identity) value;
	-- Reported as M2 reports an error, saying what was running when it came.
	reportProblem := (context, problem) -> stderr << locate problem << ": error: in "
		<< context << ": " << toString problem << endl;

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

	-- What the statement in flight showed: its drawn value, its inspection and
    -- its type annotation.
    -- The hooks only record them; the terminator sends them, because a
    -- statement that throws or ends in `;` is never recorded, and the frame
    -- must leave exactly once per cell all the same.
	renderedValue := null;
	inspection := null;
	previews := {};
	typeInfo := null;
	typeDrawing := null;

	-- The input number of the latest statement that evaluated, and whether
    -- recording it failed. BeforePrint runs for every statement that evaluated,
    -- `;` or not, once AfterEval has bound its `oN`; one that throws runs no
    -- stage at all. A number rather than a flag, so that in `x = 1; error "e"`
    -- what the first statement left cannot pass for the second's.
    --
    -- Nothing here asks what a failed statement *managed* to do before it failed,
    -- because there is no answer to that. A throw can land anywhere -- part-way
    -- through a nested parallel assignment, with some of its targets bound and
    -- the rest not -- so any reading of the session's state afterwards is a
    -- guess. These are only ever allowed to make a frame say the wrong
    -- thing, never to decide whether a frame is sent.
	evaluatedLine := null;
	failed := false;

	-- What the terminator evaluates to, so the shell's own statement is told
    -- apart by its type rather than by state carried across a statement
    -- boundary. Its hooks, installed with the protocol below, are inert: the
    -- statement reports nothing, binds no `oN`, and leaves `oo`/`ooo`/`oooo`
    -- alone. The stock AfterEval rotates that history on every statement, a
    -- null one included, so a terminator valued `null` duplicated `oo` into
    -- `ooo` after every cell.
	CellEnd := new Type of BasicList;
	CellEnd#AfterEval = identity;
	-- An annotation is a sequence of pieces -- a type, a ring, a map's shape --
    -- each of which may be drawn over several rows: `QQ^2` raises its exponent.
    -- Its text joins the pieces' nets along their baselines, as M2 prints it.
	annotationPieces := value -> select(deepSplice sequence value, item -> item =!= null);

	-- What a value is drawn as is the Spectral package's business: `rod`,
	-- which a session may install for a class of its own. The formatting hooks are
	-- taken once, before any user code runs, so rebinding a name later cannot
	-- change what is called here. `drawnAs`, `inspectedAs` and `typedAs` are
	-- `rod`, `speek` and `specType` held to giving a Rod; `forgetDrawing` clears
	-- what a drawing that threw part-way may have left behind.
	spectral := Spectral#"private dictionary";
	drawnAs := value spectral#"drawnAs";
	typedAs := value spectral#"typedAs";
	forgetDrawing := value spectral#"forgetDrawing";
	inspectedAs := value spectral#"inspectedAs";
	-- A class in a frame -- the role of a word, the kind of a list -- is sent as
	-- its ancestors, nearest first, for the shell to read by the first it knows.
	typeName := lookup(toString, Type);

	-- One frame closes one input. Marked with a control character no diagnostic
    -- and no prompt can begin with, so the shell can pick frames out of the
    -- stream by inspecting the first byte alone; printed text that happens to
    -- start with it is the user's own doing and reads back as a frame, which is
    -- the same bargain any in-band marker makes.
	frameMark := ascii 29;
	encodeFrame := value -> (
		if value === null then
			"null"
		else if instance(value, Boolean) then
			if value then
			"true"
		else "false"
		else if instance(value, ZZ) then
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
	-- Encoded whole before a byte is written, so a payload that cannot be
    -- encoded leaves nothing half-sent behind it.
	emitFrame := payload -> (
		frame := frameMark | encodeFrame payload;
		stdio << frame << endl << flush;
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
	-- A frame is the cell it closes, which reading the session's counters cannot
    -- fail to give, and the result it carries, which encoding might.
	cellFrame := (submission, error) -> hashTable {
		"submission" => submission,
		"number" => lineNumber,
		"hasValue" => OutputDictionary#?("o" | integerText lineNumber),
		"depth" => interpreterDepth,
		"error" => error
	};
	resultFrame := (submission, locals) -> merge(cellFrame(submission,
		failed or evaluatedLine =!= lineNumber), hashTable {
			"names" => newNames join(dictionaryPath, locals),
			"value" => renderedValue,
			"inspection" => inspection,
			"previews" => previews,
			"typeInfo" => typeInfo,
			"typeDrawing" => typeDrawing
		}, last);
	-- Should the result fail to encode, the cell still closes, as a failure.
	sendResult := (submission, locals) -> try emitFrame resultFrame(submission, locals)
	except problem do (
		reportProblem("sending the result", problem);
		emitFrame cellFrame(submission, true)
	);
	forgetResult := () -> (
		renderedValue = null;
		inspection = null;
		previews = {};
		typeInfo = null;
		typeDrawing = null;
		evaluatedLine = null;
		failed = false;
		forgetDrawing();
	);

	-- A result, recorded once and whole: the value under every formatter the
    -- shell offers -- Spectral's own drawing and inspection, then M2's `net`,
    -- `toString`, `tex`, `toExternalString` and `peek` as M2 gives them -- and
    -- its type, both as Spectral draws it and as M2 annotates it. Which of them
    -- the shell shows is its own business. Recording rather than printing keeps
    -- the channel free of everything but genuinely printed text.
	formatters := {
		("net", toString@@net),
		("string", toString),
		("tex", tex),
		("external", toExternalString),
		("peek", toString@@net@@peek)
	};
	recordResult := value -> (
		renderedValue = drawnAs value;
		inspection = inspectedAs value;
		previews = select(apply(formatters, (name, formatter) -> (
			text := try formatter value;
			if instance(text, String) then
				hashTable {
					"mode" => name,
					"text" => text
				}
		)), preview -> preview =!= null);
		typeDrawing = typedAs value;
		-- M2's own annotation, for the view that shows M2's own spelling.
		annotation := annotationPieces((lookup(AfterPrint, class value)) value
				?? class value);
		typeInfo = toString horizontalJoin apply(annotation, net);
	);

	-- The protocol. M2 runs BeforePrint for every statement that evaluated, then
    -- Print with its value unless that is null or a `;` silenced it, which is the
    -- same thing here: there is nothing to show. BeforePrint opens the statement,
    -- forgetting what an earlier statement of the same cell recorded, since only
    -- the last one is sent; Print records the result; every stage prints nothing.
    --
    -- Every call into session code -- `rod`, `specType`, an `AfterPrint`
    -- method -- already falls back to plain text when it throws. A recording that
    -- throws all the same is reported as M2 reports any error and marks the
    -- statement failed. Left to M2, the error would open the debugger in this
    -- file, whose frame is the protocol's own: a session typing there could
    -- rebind what every later frame is built from.
    --
    -- Only the statement the user typed is recorded. `capture` -- which
    -- `installPackage` and `check` use to run examples -- reads its input through
    -- a top level of its own under the current mode, so these hooks also fire
    -- for every statement it evaluates. Recording those would mark a cell that
    -- later throws as completed, and swap its value for an example's; while the
    -- example results, captured from what the hooks print, would lose their
    -- values. `capture` forces `stopIfError`, which the shell's own top level
    -- never sets, so under it each stage is M2's stock one instead.
	scan({BeforePrint, Print, NoPrint, AfterPrint, AfterNoPrint}, stage -> (
		CellEnd#{shell, stage} = identity;
		Thing#{shell, stage} = value -> (
			if stopIfError then
				standardStage(stage, value)
			else (
				if stage === BeforePrint then (forgetResult(); evaluatedLine =
						lineNumber);
				if stage === Print and value =!= null then
					try recordResult value
				except problem do (
					reportProblem("showing the result", problem);
					failed = true
				);
				value
			)
		)
	)
	);

	-- The prompts follow the same rule: the session shows none, while `capture`
    -- reads its examples back by the `iN :` prompts it prints, so under it they
    -- are M2's stock ones, read when asked for since M2 swaps them at will. The
    -- interpreter keys them by symbols of its own, not the `InputPrompt` Core
    -- exports, so the keys are taken from the stock prompts themselves.
	promptNames := {"InputPrompt", "InputContinuationPrompt"};
	scan(keys ZZ, key -> if instance(key, List) and first key === Standard
			and member(toString last key, promptNames) then
				ZZ#{shell, last key} = number -> (
					if stopIfError then
						ZZ#key number
					else ""
				)
	);

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
    -- the shell's `iN` contiguous. An input that never evaluated is rewound past
    -- as well, so the next cell reuses its number: nothing can refer to it, a
    -- statement that throws having bound no `oN` and left `oo` where it was. One
    -- that evaluated keeps its number even when showing it failed, its `oN`
    -- being bound already.
    -- `submission` is the shell's own count of what it has sent, echoed back so a
    -- frame that arrives for an earlier submission is recognised and dropped
    -- instead of being filed under the cell now waiting.
    -- Keep interruption from leaving a partial frame or half-reset cell state.
	User#"spectral end of cell" = (submission, locals) -> shield (
		lineNumber -= 1;
		lastEvaluated := evaluatedLine === lineNumber;
		-- What an earlier statement of the cell recorded is not this input's.
		if not lastEvaluated then
			forgetResult();
		sendResult(submission, locals);
		if not lastEvaluated then
			lineNumber -= 1;
		forgetResult();
		new CellEnd
	);

	-- The shell's own init file, loaded once the hooks it may edit exist and
    -- before the session opens, so what it prints arrives as startup output.
    -- One that throws is reported, and the session opens all the same.
	if User#?"spectral init file" then
		try load User#"spectral init file"
	except problem do
		reportProblem(User#"spectral init file", problem);

	-- Announced once the hooks are installed and before the first cell can be
    -- read, so the shell knows the child is up, which M2 it got, and every name
    -- the session starts with.
	emitFrame hashTable {
		"ready" => version#"VERSION",
		"spectral" => (options Spectral).Version,
		"names" => newNames dictionaryPath
	};
	shell
))()

-- Last, so that nothing above is echoed: from here the channel carries only what
-- the session produces.
echoOff()
topLevelMode = spectralShell

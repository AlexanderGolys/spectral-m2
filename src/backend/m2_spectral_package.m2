-- Spectral, the package every result is drawn with: installed when missing, then
-- loaded. Until Macaulay2's own collection carries it, it is fetched from its
-- repository; from then on `installPackage "Spectral"` alone will do.
spectralRepository := "https://github.com/AlexanderGolys/Spectral"

-- A session started with -q leaves out the user's own packages, where Spectral
-- is installed, so they are put back: their prefix, and the files' directory.
userPrefix := applicationDirectory() | "local/"
prefixPath = unique prepend(userPrefix, prefixPath)
path = unique prepend(userPrefix | Layout#1#"packages", path)

-- Fetching and installing write their progress to the session's own output, so
-- both run apart, quietly; installing renders the pages with the package loaded
-- by name, so its clone leads the path there.
quietly := command -> run(command | " >/dev/null 2>&1") == 0
if not member("Spectral", installedPackages()) then (
	clone := temporaryFileName();
	install := concatenate("path = prepend(", format(clone | "/"),
		", path); installPackage \"Spectral\"; exit 0");
	if not quietly("git clone --quiet --depth 1 " | spectralRepository | " " | clone) then
		error("cannot fetch Spectral from " | spectralRepository);
	if not quietly(commandLine#0 | " --silent -q --stop -e '" | install | "'") then
		error("cannot install Spectral fetched from " | spectralRepository)
)

needsPackage("Spectral", LoadDocumentation => true)

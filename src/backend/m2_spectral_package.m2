-- Spectral, the package every result is drawn with, loaded as any package is:
-- from the user's Macaulay2, where their own code can load it too and install
-- `rod` or `formatted` for types of their own. The shell needs a version that
-- sends what it reads, and carries one, written beside this file, which it
-- installs first when the user's Macaulay2 has none as recent.
carried := currentFileDirectory | "Spectral.m2"
-- The first version sending drawings as the shell reads them; raised only when
-- what Spectral sends changes in a way the shell cannot read.
minimumVersion := "0.7.0"

-- A session started with -q leaves out the user's own packages, where Spectral
-- is installed, so they are put back: their prefix, and the files' directory.
userPrefix := applicationDirectory() | "local/"
prefixPath = unique prepend(userPrefix, prefixPath)
path = unique prepend(userPrefix | Layout#1#"packages", path)

-- A version as the list of its numbers, which compare as versions do:
-- "0.10" is {0, 10}, after {0, 9}.
versionNumbers := version -> value \ separate("[.]", version)
readable := member("Spectral", installedPackages()) and (
	(try versionNumbers (readPackage "Spectral").Version
			else {})
		>= versionNumbers minimumVersion
)

-- Installing writes its progress to the session's own output, so it runs apart,
-- quietly, from the carried package's directory: its documentation reads the
-- package's header by file name, which a session started with -q finds only
-- in the directory it runs in.
if not readable and run concatenate("cd ", format currentFileDirectory, " && ",
	commandLine#0,
	" --silent -q --stop -e 'installPackage(", format "Spectral", ", FileName => ",
	format carried, "); exit 0' >/dev/null 2>&1") != 0 then
	error("cannot install Spectral from " | carried)

needsPackage("Spectral", LoadDocumentation => true)

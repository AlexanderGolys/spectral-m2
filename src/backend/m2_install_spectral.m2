-- Installs the Spectral package the shell ships, the file given as the script's
-- argument, when none as recent is installed, so M2's own documentation (`help`,
-- `viewHelp`, the language server) covers the version the shell runs. A copy as
-- new or newer, such as one installed from Spectral's own repository, stays.
--
-- Run as `M2 --script`, which leaves the home directory's packages out; they are
-- where an installed copy lives and where this one goes, so they come back first.

prefixPath = prepend(applicationDirectory() | "local/", prefixPath)
shipped := scriptCommandLine#1

outdated := (not member("Spectral", installedPackages())
		or (readPackage "Spectral").Version < (readPackage("Spectral",
			FileName => shipped)).Version)

-- Pages are rendered with the package loaded by name, so its directory leads the path.
if outdated then (
	path = prepend(replace("[^/]*$", "", shipped), path);
	installPackage "Spectral"
)

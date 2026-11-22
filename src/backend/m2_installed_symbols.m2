-- Which installed package documents each symbol a fresh session leaves unbound,
-- printed as one `name package` line per symbol. Read from the documentation
-- each package installs beside itself, so no package is loaded to tell.
--
-- A documentation database holds a raw entry per documented key: a symbol's
-- entry stores its Key bare, as `Key => Digraph`, while a topic page's Key is
-- a string, as `Key => "gfan"`. Only the former names something a
-- `needsPackage` binds.
(() -> (
	  database := "rawdocumentation-" | version#"endianness" | "-"
	  | version#"pointer size" | ".db";
	  scan(prefixPath, prefix -> scan(values Layout, layout -> (
			 packages := prefix | layout#"packages";
			 if isDirectory packages then
			 scan(readDirectory packages, file -> if match(///\.m2$///, file)
			      then (
				   package := replace(///\.m2$///, "", file);
				   path := prefix | replace("PKG", package,
					layout#"packagecache") | database;
				   if fileExists path then (
					entries := openDatabase path;
					scan(keys entries, key -> if match(///^[[:alpha:]][[:alnum:]']*$///, key)
					     and not isGlobalSymbol key
					     and match("Key => " | key | "[,}]", entries#key) then
					     print(key | " " | package));
					close entries
					)
				   ))
			 )))
	  ))()

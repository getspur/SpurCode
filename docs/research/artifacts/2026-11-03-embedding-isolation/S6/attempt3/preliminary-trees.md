# Preliminary dependency trees

Source HEAD: `94d727e4d96c955169c36cc5647cf6970f2b0386`. These are not final S6 acceptance; repeat after owner and host changes.

All four `normal,build` trees and all four `features,no-dev` trees exited 0 and contained no `fastembed`, `ort`, `ort-sys`, `owner-test-hooks`, or `test-support` lines under the scan recorded in results JSON. Unfiltered feature trees are included and contain test-support feature lines for core and CLI because they include development edges; they are not normal dependency closures.

Commands, raw stdout/stderr and summary JSON are retained in `preliminary-trees.tar.gz`.

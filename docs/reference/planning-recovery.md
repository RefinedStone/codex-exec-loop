# Diagnose an occupied legacy planning namespace

Run this command from the Git checkout you want to inspect:

```text
akra planning-recovery diagnose [workspace_dir]
```

Set `AKRA_HOME` to the absolute path of the profile used by the old release if it
is not the current profile. For an unsupported linked-worktree layout, run the
command from the primary checkout.

`legacy_namespace: occupied` means an entry exists at the historical v1.3.5
project path. It does not mean that the entry contains a database, or that it
belongs to this checkout. A different Git repository can reuse the same path.
Keep the historical directory unchanged. The command does not authorize
recovery or import.

`legacy_namespace: overlaps_current_namespace` means the current incarnation
marker names the same path. This is still a path observation, not a report about
the database or its owner. `legacy_namespace: absent` means no historical entry
was found for this checkout in the selected profile. Check the old profile
before concluding that the data is missing.

The command reads Git metadata, an existing incarnation marker if needed, and
directory metadata. It does not initialize configuration, create a marker, or
open, copy, select, or modify a planning database. A successful exit means the
inspection completed, not that a database is safe to use.

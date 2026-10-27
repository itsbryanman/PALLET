# Feature status

| Feature | Status | Safety clauses |
| --- | --- | --- |
| Virtual browser sample | Available; never accesses local files | S1, S7 |
| Linux real-file delivery | Enabled only on ext4 with Landlock and required kernel support | S1–S5, S7 |
| Undo move | Conditional no-replace reverse rename | S2, S3, S6 |
| Duplicator | Verified same-root copy; incomplete temporary output is hidden | S2, S6 |
| Incident reports | Explicit no-overwrite files inside the warehouse | S1, S2, S6 |
| Windows real files | Disabled; native no-replace handle rename is proved, production confinement adapter is in progress | S1–S6 |
| macOS real files | Disabled pending native adapter and test evidence | S1–S6 |

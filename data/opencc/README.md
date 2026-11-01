# OpenCC traditional-to-simplified tables (vendored)

`TSCharacters.txt` and `TSPhrases.txt` are copied verbatim from
[OpenCC](https://github.com/BYVoid/OpenCC) tag `ver.1.4.2`:

- `data/dictionary/TSCharacters.txt`
- `data/dictionary/TSPhrases.txt`
- `LICENSE` (Apache License 2.0)

They drive `dict-tools compile --simplify`; the runtime engine has no
OpenCC dependency. Conversion follows the `t2s.json` chain: longest phrase
match first, then the character table. Only the first value on each line is
used, matching OpenCC's default candidate.

Pinned hashes (sha256):

```
737c21c66f55a419dd6956cb3089476cdefc5a36877452631617696df1e5d925  TSCharacters.txt
362fa1b9a7d6edd04b462a32e12c9fef3adae822ab1dee9c83561cc37c06cb1f  TSPhrases.txt
b534e465949558eec2597b04f5092b5e161236a68dfbfd04d547592ac3964308  LICENSE
```

Source URL pattern:
`https://raw.githubusercontent.com/BYVoid/OpenCC/ver.1.4.2/data/dictionary/<file>`.

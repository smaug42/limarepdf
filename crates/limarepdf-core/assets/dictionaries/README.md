# Spelling dictionaries

The dictionaries spelling checks words against (`src/spelling.rs`), in Hunspell's format: an
`.aff` file of rules and a `.dic` file of words. All are unmodified, as released.

## English

`en_CA`, `en_US`, `en_GB-ise` and `en_AU` are SCOWL's Hunspell dictionaries, version 2020.12.07
(Kevin Atkinson and others; http://wordlist.aspell.net), from the release archives
`hunspell-en_CA-2020.12.07.zip`, `hunspell-en_US-2020.12.07.zip`,
`hunspell-en_GB-ise-2020.12.07.zip` and `hunspell-en_AU-2020.12.07.zip` on SourceForge
(`wordlist/speller/2020.12.07/`). They are SCOWL's normal size (60), one spelling of each word:
Canadian English as SCOWL has it (-our, -re and -ize: "colour", "centre", "organize"), British
English with -ise. Their copyright notices and permission notice, a permissive licence that asks
only that the notices be kept, are in `README_en_CA.txt` (the four archives' READMEs differ only
in their first line); release archives include it as `DICTIONARY-LICENSE-English.txt`. The British
dictionary LibreOffice ships (Marco A.G. Pinto's) is LGPL, so SCOWL's is used instead.

| Archive | sha256 |
| --- | --- |
| hunspell-en_CA-2020.12.07.zip | `ff6b91e4ed768348c61ae7c326e848059810fa43a5d601df6b3f45ad9c0ef5bf` |
| hunspell-en_US-2020.12.07.zip | `616348ad645a716d91c8a6645065e710f15e9dda3ffef60cdf7ec8a4e27975af` |
| hunspell-en_GB-ise-2020.12.07.zip | `2ced4b2ec9d188c3b540dbb75b5143f65b990daf17437758726ff1fab5f01ad3` |
| hunspell-en_AU-2020.12.07.zip | `dc20557c48ae1979784e79fae6f965e999c8db2e9a0f846348e70057fce78254` |

## French

`fr.aff` and `fr.dic` are Grammalecte's French dictionary 7.7, "Classique" (Olivier R. and its
contributors; https://grammalecte.net), as LibreOffice ships it for every French locale, Canada's
among them: `fr_FR/dictionaries/` in https://github.com/LibreOffice/dictionaries at commit
`fe691977c619b2ca764531b3507e83fab4732651`. It is licensed under the Mozilla Public License 2.0
(`LICENSE-MPL-2.0.txt`, from https://www.mozilla.org/media/MPL/2.0/index.txt); its README,
`README_dict_fr.txt`, says so and where its source is. Release archives include them as
`DICTIONARY-LICENSE-French.txt` and `LICENSE-MPL-2.0.txt`. The MPL-2.0 also covers the `spellbook`
crate that reads the dictionaries (https://github.com/helix-editor/spellbook), used unmodified.

## Files

| File | sha256 |
| --- | --- |
| en_CA.aff, en_US.aff, en_GB-ise.aff, en_AU.aff | `c7a8c4d08c29d237880844b1623099f59092602f189be38ce3912e457ff38bc1` |
| en_CA.dic | `53096689870246a78fa408b1e7a8c49bc140bd466a84a384cfb509445fff7173` |
| en_US.dic | `f0b1a234bd178bdd01875b2a392a9647f888b8fe879f79c52aae62c2759b3647` |
| en_GB-ise.dic | `869fe17ba4ee4b5401c60a666ee2d6a3dcc237f460b7294435df1cc6a799aa57` |
| en_AU.dic | `aa07c46571f306b79fc1bc534357ed357af15687381b26f891ba66e8a2caed89` |
| fr.aff | `c176610cd5dc4846806a65ddd029f422d87978bf58f224aa44222662a16a2de5` |
| fr.dic | `b78a868e31dd6e373b6c3217969afb898a9acde828a5e7ef97308da42218c88c` |
| README_en_CA.txt | `86700e39b2307dda4c087ad3505a1e634a5b4d518ab808c398a16d21ebe957e2` |
| README_dict_fr.txt | `4a25abf740ad6ba5e8d0cfc0d2ffadc3c13c303f8fe389f323b061a389350f83` |
| LICENSE-MPL-2.0.txt | `3f3d9e0024b1921b067d6f7f88deb4a60cbe7a78e76c64e3f1d7fc3b779b9d04` |

They are embedded in `limarepdf-core` compressed (about 1.1 MB instead of 3.7 MB): `build.rs`
packs each with zlib, and `src/spelling.rs` unpacks one the first time its language is checked,
byte for byte as it is here; a test (`tests/spelling.rs`) checks each unpacked file against the
file here. The files here stay as released.

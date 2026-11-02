# Bundled interface fonts

`Inter-Regular.ttf` (400) and `Inter-SemiBold.ttf` (600) are the unmodified,
full static TrueType files from the official Inter 4.1 release:

https://github.com/rsms/inter/releases/tag/v4.1

Archive: `Inter-4.1.zip`, members `extras/ttf/Inter-Regular.ttf` and
`extras/ttf/Inter-SemiBold.ttf`. The accompanying SIL Open Font License is
in `LICENSE-Inter.txt` and is included in the macOS app bundle.

SHA-256:

- Regular: `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82`
- SemiBold: `78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3`

Both files contain Latin, Cyrillic and Greek glyphs. Keep Cyrillic in both
weights: Latin-only subsets make Russian text fall back to egui's regular
font even in headings and buttons, losing the intended hierarchy. The
font-coverage test in `src/ui/app.rs` guards against this regression.

Fredoka is used for the Latin SquirrelDisk wordmark. Other scripts fall back
to Inter and the existing symbol/system fonts.

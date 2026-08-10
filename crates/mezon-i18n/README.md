# mezon-i18n

Localization for Mezon Desktop. Translation data lives in `assets/i18n/<locale>.json`
(one flat `key → value` file per locale). Keys follow the same dot-key convention as
mezon-react (`namespace.key.path`) from the initial web corpus import; **web and
desktop strings are maintained independently** — edit the JSON files here directly.

## TL;DR

- Translations live in `assets/i18n/<locale>.json` (workspace root), one flat
  `key → value` file per locale.
- Look up a string with `mezon_i18n::t(&locale, "namespace.key.path")`.
- **Edit a JSON file → rebuild (`just run`).** Cargo auto-detects the change; there
  is no codegen step.
- To add a brand-new string, add it to **`en.json` at minimum** (it is the
  fallback), then use it in code.

## Layout

| Path | What |
|------|------|
| `assets/i18n/<locale>.json` | The strings. One file per locale, flat dot-keys, UTF-8, sorted. |
| `crates/mezon-i18n/src/lib.rs` | The runtime: `t()` + the per-locale loader. |
| `crates/mezon-ui/src/settings/language_page.rs` | The language picker screen (`LANGUAGES` list). |
| `crates/mezon-ui/assets/icons/flags/<locale>.svg` | Flag shown per language. |

Supported locales: `en` (default + fallback), `vi`, `ru`, `es`, `tt`, `de`, `it`,
`pt`, `jpn`, `kr`, `swe`.

## How lookup works

```rust
pub fn t(locale: &str, key: &'static str) -> &'static str
```

Each locale's JSON is embedded at compile time (`include_str!`) and parsed **once,
lazily** into a `static OnceLock<HashMap<String, String>>` on first use. Values are
returned as `&'static str` (borrowed from the static map — no per-call allocation).

Resolution order: requested locale → English fallback → the key string itself (so a
missing key renders as e.g. `"clan.title"`, which is easy to spot).

There is **no `build.rs`**. A code-generated `match` does not scale to ~45k arms
(≈4.2k keys × 11 locales); runtime lazy-parse compiles fast and is O(1) at lookup.

## Key format

React's nested `namespace:key.path` is flattened to dot-keys:

| React | Desktop key |
|-------|-------------|
| `t('clan:title')` | `clan.title` |
| `t('setting:language.title')` | `setting.language.title` |
| array element `monthsShort[0]` | `channelCreator.monthsShort.0` |
| object in array `faq.questions[0].question` | `clandetail.faq.questions.0.question` |

So when migrating a React component, change `t('ns:key.path')` →
`mezon_i18n::t(&locale, "ns.key.path")`.

## Adding / changing a key

1. Add or edit the key in `assets/i18n/en.json` (**required** — English is the
   fallback). Add it to other locale files as you translate them; any locale missing
   the key falls back to English automatically.
2. Use it: `mezon_i18n::t(&locale, "your.new.key")` (the key must be a string
   literal).
3. Rebuild — see below.

When a string should match the web app, update `assets/i18n/*.json` here and
`mezon/libs/translations` in the React repo in separate PRs — there is no
automated sync step.

## Rebuilding (yes, it is required)

Strings are compiled into the binary, **not** read from disk at runtime. Editing a
JSON file therefore needs a rebuild — but Cargo tracks `include_str!` dependencies,
so it recompiles `mezon-i18n` (and dependents) automatically. No manual codegen.

```sh
just run                 # build + run the app   (or: cargo run -p mezon-app)
just check               # fast clippy           (or: cargo check -p mezon-i18n)
just test -p mezon-i18n  # run the i18n tests
```

Incremental, so it is fast — only `mezon-i18n` and crates that use it recompile.

> No hot-reload by design (the binary is self-contained). If live-editing
> translations without a rebuild ever becomes worth it, switch the `include_str!`
> loaders in `lib.rs` to read the JSON from disk at startup.

## Adding a new locale

1. Create `assets/i18n/<code>.json` by hand (copy an existing locale as a template).
2. Add a match arm in `crates/mezon-i18n/src/lib.rs` → `data()`:
   ```rust
   "<code>" => load!("../../../assets/i18n/<code>.json"),
   ```
3. To make it selectable in the UI, add an entry to `LANGUAGES` in
   `crates/mezon-ui/src/settings/language_page.rs` and a flag at
   `crates/mezon-ui/assets/icons/flags/<code>.svg`.
4. Rebuild.

## Flags

Flags are **multi-color** SVGs, rendered with `img("icons/flags/<code>.svg")` — not
the `Icon`/`svg()` element, which is a single-color alpha mask. The flag files are
embedded by `mezon-ui`'s `Assets` (`#[include = "icons/**/*.svg"]`).

## Tests

`crates/mezon-i18n/src/lib.rs` has unit tests covering dispatch, English fallback,
unknown-key passthrough, that every locale bundle parses, and that the full React
corpus is present. Run `cargo test -p mezon-i18n`.

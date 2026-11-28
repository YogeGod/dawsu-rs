# Mapset Verifier oracle

This tool checks that dawsu! output never adds a [Mapset Verifier](https://github.com/Naxesss/MapsetVerifier) (MV) ranking issue. MV checks each map before and after dawsu! changes it. A case fails when the "after" map has a new `Problem`, `Warning` or `Error` ("unable to check"). New `Minor` issues show only in the report.

**Licence.** MV is GPL-3.0. It is not vendored, built into dawsu!, or shipped. The oracle runs MV as a separate local process over loopback HTTP, and only reads its JSON. Do not copy MV source or message text into dawsu! code.

**Pin.** The oracle uses MV commit `7a4c18dce340d4c373e8ac7d61125f6bf8f9d8df`, set as `MV_SHA` in `oracle.mjs`. When you move the pin, re-run every case and commit `expected-new-issues.json` together with the new pin.

## Run

1. Build MV once (about 1 min; the first build needs network for NuGet):
   `node tools/mv-oracle/build.mjs`
   This reads `E:/dawsu-source-cache/Naxesss-MapsetVerifier-7a4c18d` and writes `E:/dawsu-build/mv-oracle/7a4c18d/MapsetVerifier.exe`.
2. Write the cases (about 1 min):
   `cargo test -p dawsu-application --locked --test mv_cases -- --ignored --nocapture`
3. Check the cases (about 30 s):
   `node tools/mv-oracle/run.mjs`

Exit codes: `0` means every case passed. `1` means at least one case failed. `2` means the run was skipped, because there is no MV build or there are no cases. Add `--allow-skip` to make a skip exit `0`.

The run prints one line per case and ends with `mv oracle: N of M cases passed`. The full report goes to `target/mv-oracle/report.json`, with the issues that made each case fail and the checks that ran.

## Cases

`crates/dawsu-application/tests/mv_cases.rs` copies each input set twice, into `before/` and `after/`, then runs one dawsu! write path on `after/`.

- **Inputs:**
  - each `.osu` from `tests/fixtures/save-pack` and `tests/fixtures/oracle-cases`, each in its own folder;
  - `tools/make-demo-mapset.mjs`;
  - up to 20 sets copied from `%LOCALAPPDATA%/osu!/Songs`. Set `MV_SONGS_DIR` to use another folder and `MV_SONGS_LIMIT` to change the count. The originals are only read.
- **Write paths:**
  - `save`: every difficulty, no edit.
  - `edit`: the same clap is added at the first object, in every difficulty that has an object there.
  - `hsspread`: dawsu! makes a `[Hitsounds]` difficulty, copies it to every difficulty, then deletes it.
- **Refused cases:** when dawsu! refuses an input (non-standard mode, or fixes needed first), the case is removed and listed in `manifest.txt`.

## Results

- **before fails** (HTTP error or timeout): the case is a `SKIP`, because the input is broken.
- **after fails:** the case is a `FAIL`.
- **Timeouts:** each request has a 120 s timeout. After a timeout, MV restarts.

## Isolation

MV runs with `MAPSET_VERIFIER_CUSTOM_CHECKS_ENABLED=false`, its own `TEMP`/`TMP` under `E:/dawsu-build/tmp`, `CreateSnapshot=false` and `IncludeCheckRunDelta=false`. MV still writes its own log to `%APPDATA%/Mapset Verifier Externals/Logs`.

## Accepted new issues

`expected-new-issues.json` lists accepted new issues as `[{ "case": "<name or *>", "key": "<issue key from the report>", "reason": "..." }]`. Each entry cancels one occurrence. Every entry needs the owner's OK.

## Tests

Run `node --test tools/mv-oracle/oracle.test.mjs` to test the issue keys, the before/after count diff and the case results.

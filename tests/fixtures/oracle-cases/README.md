# Hand-made lazer oracle cases

Small maps written for the lazer oracle, committed with their results in `tests/fixtures/lazer-oracle/oracle-cases/` (ADR 0039). They pin rules the save pack does not cover.

- `before-first-line.osu`: a slider before the first timing line, which plays at slider velocity 1 with the first red line's beat length.
- `at-red-and-green-pair.osu`: a slider exactly at a red line and a green line (x0.6) at the same time, where the green line wins.
- `after-red-following-green.osu`: a slider after a green line (x0.5), and one after a later red line, which resets the velocity to 1.
- `green-before-first-red.osu`: a slider before a green line placed before the first red line (velocity 1), and one after it (velocity 0.6 with the red line's beat length).

The rules are in "lazer facts for M2" in the design vault (`dawsu-rewrite/research/osu-rules/lazer-facts-for-m2.md`).

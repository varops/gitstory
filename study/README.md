# Study tools

Tools for the M0 quality study (PRD §13). They measure GitStory; they are not part of using it, so the commands are hidden from `--help`.

Every rate is printed with its 95% Wilson interval. Overlapping intervals mean the runs do not tell two rates apart. Report the interval, never the fraction alone.

## 1. Validator audit: does validation catch false claims?

[`validator-audit/corpus.json`](validator-audit/corpus.json) holds realistic cases: durable evidence, a diff, and claims whose truth is known. Most claims are true. The rest are planted, each labeled with how it is false:

| Plant | Example |
|---|---|
| `wrong_number` | "a 60s lease" when the evidence says 30s |
| `wrong_reason` | the right decision with a reason nobody gave |
| `invented` | a change that is not in the evidence or the diff |
| `contradicted` | the evidence says the opposite |
| `overreach` | "every endpoint" when the change touched one |
| `outside_knowledge` | true in the world, but not in the evidence |
| `wrong_attribution` | "the agent decided" when a reviewer did |
| `rejected_as_implemented` | a dropped approach presented as the decision |
| `test_misread` | "tests passed" when 47 of 50 did |

Run it in any repository with `git story init` done. It uses that repository's validation model and egress policy:

```bash
git story study audit study/validator-audit/corpus.json
git story study audit study/validator-audit/corpus.json --json > audit.json
```

It runs the same steps as landing (local rules, then the validation pass with its repair step) and reports:

- **planted claims caught:** dropped, moved to hypotheses, or rewritten by repair. This is the number behind "every claim is cited".
- **planted claims dropped:** caught and removed outright.
- **true claims lost:** moved to hypotheses or dropped. The cost side: a validator that drops everything catches every plant.
- **claims repaired:** each rewrite is listed with its old and new wording. Check that every rewritten planted claim really lost its falsehood.
- a breakdown by kind of plant, and every miss.

Each run is scored against two targets: at least 95% of planted claims caught, and at most 5% of true claims lost. Change them with `--min-caught 0.9 --max-lost 0.1`. A target reads **shown** when the whole interval is on the right side, **not yet shown** when the rate meets it but the corpus is too small to be sure, and **missed** when the rate itself is on the wrong side. The overall verdict is the weaker of the two: **shown** only when both are shown. Both targets matter: catching everything is easy if you also lose every true claim. With the shipped corpus (21 planted, 16 true claims) neither target can be shown even by a perfect run; the intervals are too wide. Even a perfect run needs at least 75 planted and 75 true claims to show them.

### The repair step

When the validation model says a claim is only partly supported, its note says what is missing. Landing then asks the synthesis model once to rewrite the claim using only its cited evidence and the diff, and judges the rewrite again. The rewrite must keep the claim's meaning (a reason stays a reason, an outcome stays an outcome). When the evidence supports the meaning but not the wording, the claim is reworded; when it does not support the meaning, the model gives up instead of swapping in a weaker fact. A rewrite that passes replaces the claim; anything else moves to hypotheses as before. This saves true claims that were worded beyond their evidence (for example "64 MiB" where the diff says `64 * 1024`). The repair and second check are recorded in the egress manifest with purposes `repair` and `revalidation`, and only run when a claim fails.

The fixture data goes to the model through the egress gate like any other call. The manifest is in the `--json` output; nothing is written to the store.

To add a case, copy one, keep true claims in the majority, and label every plant. Claims in one case must have distinct text.

## 2. Control arm: does GitStory beat one plain model call?

Without a control, a good rating means nothing: the model might write just as good an explanation from the same evidence on its own.

When you land a commit for the study, add `--study-baseline DIR`:

```bash
git story land --work-unit TCK-482 --to <sha> --study-baseline ~/m0
```

Landing works as usual. In the same run, the same evidence events and diff go to the same model in one plain call: no drafts, no rules, no validation. The call goes through the same egress gate and is recorded in the same manifest, with purpose `study_baseline`.

It writes:

- `DIR/<commit>.md`: a rating sheet with both write-ups as **Version A** and **Version B**, in an order raters cannot predict. The GitStory side shows claims only, with no evidence ids or claim types.
- `DIR/key.jsonl`: which version is which, one line per commit. Keep it away from raters.

Raters read the change, then fill one row per sheet in `DIR/ratings.csv`:

```csv
commit,rater,a_accuracy,a_usefulness,b_accuracy,b_usefulness,prefer
8e79d6dfe3a1,ran,5,4,3,3,A
```

Accuracy and usefulness are 1–5. `prefer` is `A`, `B` or `same`. Then:

```bash
python3 study/m0/score.py ~/m0
```

It un-blinds the labels and prints, per arm, the mean scores and the share rated 4–5, and the share of decided preferences that went to GitStory, each with its interval.

**Limits.** Blinding is partial: GitStory's write-up is structured and the baseline is prose, so a rater can often guess. Accuracy should still be judged against the change itself. Twenty commits give wide intervals; that is the honest size of what M0 can show.

## Known gaps

Fix first. A known bug goes into [`cli/tests/known_gaps.rs`](../cli/tests/known_gaps.rs) only when its fix has to wait, pinned by a test that passes while the bug is open. Fixing it makes that test fail; then flip the assertion and move the test to where the feature is tested.

The audit's first run found two validator bugs, both fixed: a claim the model never ruled on was kept as supported, and a test claim with matching counts was accepted whatever else it said. Their tests are in [`cli/tests/validation.rs`](../cli/tests/validation.rs).

# Validator audit results

**Every claim in a GitStory story must be supported by its evidence. This audit measures how well the validator enforces that.** We planted 119 false claims among 111 true ones and ran the real validation pass over them with four models. Every model caught at least 95% of the false claims and lost at most 5% of the true ones, **shown at 95% confidence**.

Measured on 2026-11-04 at commit `2600f2e`, corpus `study/validator-audit/corpus.json`.

## Results

| Validation model | False claims caught | True claims lost | Removed outright | Repaired | Targets |
|---|---|---|---|---|---|
| Claude Opus 5.5 | **119/119** [0.969, 1.000] | **0/111** [0.000, 0.033] | 101 | 6 | shown |
| gpt-6-sol, run 1 | **119/119** [0.969, 1.000] | **0/111** [0.000, 0.033] | 87 | 24 | shown |
| gpt-6-sol, run 2 | **119/119** [0.969, 1.000] | **0/111** [0.000, 0.033] | 85 | 24 | shown |
| Claude Haiku 5.5 | **118/119** [0.954, 0.999] | **0/111** [0.000, 0.033] | 92 | 15 | shown |

Brackets are 95% Wilson intervals. A target is **shown** only when the whole interval is on the right side of it: at least 95% of false claims caught, at most 5% of true claims lost.

- **Caught** means the false claim did not survive as an authoritative claim. It was removed, moved to hypotheses (which are never presented as decisions), or rewritten by the repair step so the false part is gone.
- **Repaired** claims are rewritten from their own evidence and checked again. Every rewrite of a planted claim was checked automatically for its labeled false part and also read by hand. None kept it, except the one Haiku miss below.
- **The one miss:** Haiku repaired "The agent decided to rehash on login to avoid a forced password reset" into "The agent decided to rehash on login because existing bcrypt hashes must keep working". The reason is now true, but the decision came from a constraint, not the agent.

Opus removes most false claims outright and rarely needs repair. gpt-6-sol leans on the repair step more. Both end up in the same place.

## What is in the corpus

21 realistic cases in 14 languages and formats: bug fixes, refactors, dependency swaps, config, migrations, performance, security and test-only changes. Each case has typed evidence events, a diff, and claims whose truth is known.

The 111 true claims include reworded ones and unit changes (`64 * 1024` KiB as "64 MiB"), to measure how often the validator rejects the truth. The 119 planted claims come in 14 kinds, each with its false part labeled:

| Kind | Example | Count |
|---|---|---|
| wrong_number | "a 60s lease" when the evidence says 30s | 13 |
| one_word | "increased" for "decreased" | 12 |
| contradicted | the evidence says the opposite | 10 |
| test_misread | "tests passed" when 47 of 50 did | 10 |
| false_because | two true facts joined by a "because" nobody stated | 9 |
| outside_knowledge | true in the world, but not in the evidence | 9 |
| wrong_attribution | "finance asked" when the tech lead did | 8 |
| wrong_citation | a true statement citing evidence that does not support it | 8 |
| overreach | "in production" when it was measured on staging | 8 |
| invented | a change that is not in the evidence or the diff | 7 |
| partly_false | a long claim with one false part | 7 |
| wrong_reason | the right decision with a reason nobody gave | 6 |
| stale | true of an earlier attempt, false of the landed code | 6 |
| rejected_as_implemented | a dropped approach presented as the decision | 6 |

## How it was run

`git story study audit study/validator-audit/corpus.json` runs the same steps as landing: local rules, then the validation pass with its repair step. gpt-6-sol ran through GitStory's built-in `openai` provider at its default effort. The Claude models ran through the `command` provider, wrapping the `claude` CLI with structured output and no tools. Each run made 21 validation calls plus one repair and one revalidation call per case that needed them: 42 to 57 calls per run.

The method, targets and fates are explained in [`study/README.md`](../README.md). To check a model of your own, configure it in `.gitstory` and run the command above.

## Limits

- **This is our own synthetic corpus**, not real stories. It shows the validator enforces evidence on realistic claims. Whether stories built from real sessions are accurate and useful is what the M0 study measures next.
- **One run per model, two for gpt-6-sol.** Model answers vary between runs, and earlier rounds on draft versions of this corpus had occasional misses. We report the final runs as measured and do not pool them, since they reuse the same claims.
- **The corpus was fixed during development.** When a checker's note showed a claim was labeled wrong (for example, a "true" claim whose evidence did not fully support it), we fixed the claim, never to raise a score. All four runs above use the final corpus.
- **The false-part check is a text match.** It can miss a falsehood reworded in a repair, so every repaired planted claim was also read by hand.
- **The validator found real bugs on the way here.** Unjudged claims passed, matching test counts overrode the model, verdicts were matched by index (gpt-6-sol swapped two), and the repair step used the diff for claims that never cited code. All are fixed and covered by tests in `cli/tests/validation.rs`.

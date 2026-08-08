# Competitive Research Methodology

This contract keeps the research current without turning `docs/` into a second copy of every
competitor repository.

## Evidence Classes

| Class | Meaning | Allowed claim |
| --- | --- | --- |
| `verified/source` | inspected immutable source at the recorded commit | current implementation at that pin |
| `verified/runtime` | reproduced against a named binary or local installation | observed behavior in that environment |
| `documented` | official release notes or product documentation | vendor-documented behavior |
| `inferred` | conclusion from multiple verified facts | interpretation, labeled as such |
| `proposed` | experimental, future, beta, or unreleased path | direction only |
| `unverified` | marketing, stale secondary material, or failed reproduction | limitation, never scored as shipped |

One claim can carry more than one class. A local dev checkout is not a released product even when
its source is public.

## Required Snapshot

Every product brief records:

- product and official repository/site;
- release/tag and full source commit, or a closed-source limitation;
- release and audit dates;
- Akra baseline;
- runtime environment when runtime evidence is used;
- previous Akra pin and material delta;
- evidence gaps that could change the decision.

Source links should include the full commit. Release tags alone are insufficient when an annotated
tag or mutable branch obscures the source object.

## Refresh Procedure

1. Read the existing conclusion and identify claims that affect an Akra decision.
2. Resolve the latest stable release and dereference its tag to a commit.
3. Inspect release notes and the source paths behind material changes.
4. Reproduce only behavior that matters and can be tested safely.
5. Rewrite the compact brief; do not append a second chronological report.
6. Check links and repository tests. Git history is the archive.

Refresh when a material release invalidates a conclusion, a product enters the active roadmap, or
the pin is older than 90 days. A version bump with no decision impact needs only a pin check.

## Installed-Product Evidence

Installed software can disambiguate similarly named products and verify the path actually in use.
The audit may read:

- package name and version;
- public repository URL and source commit;
- executable or adapter identity;
- aggregate numeric usage fields needed for a stated metric.

It must not copy prompts, responses, credentials, account identifiers, session identifiers, private
repository names, or unrestricted configuration. Report a sanitized aggregate and a reproducible
method instead of checking raw logs into the repository.

## Runtime Measurements

A measurement states:

- exact numerator and denominator;
- sample count and time window;
- model/provider mix when available;
- cold/warm and restart conditions;
- whether values are provider-reported or inferred;
- confounders and missing telemetry.

Cache-hit rate, latency, context pressure, provider spend, and total token volume are separate
metrics. See [Cache and Token Efficiency](cache-and-token-efficiency.md) for normalization.

Synthetic/mock-provider tests verify plumbing, not a production cache. A rolling local snapshot is
not a benchmark and must not become a product target without a controlled Akra comparison.

## Decision Format

Each brief ends with:

- **Adopt:** a narrow behavior compatible with Akra's architecture;
- **Reject:** scope or mechanism Akra should not own;
- **Differentiate:** an Akra capability that remains structurally valuable;
- **Watch:** evidence that could change the decision.

Implementation slices belong in an explicitly proposed plan or issue after product review. The
competitive brief records the reason, not a permanent backlog.

## Retention

Keep only the current product brief and reusable evidence that still supports tests or reproduction.
Delete completed interactive reports, duplicate translations, generated screenshots, and raw
command output after their conclusion is integrated. All tracked deletions remain recoverable from
Git history.

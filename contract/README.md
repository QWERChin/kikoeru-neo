# API contract samples

These synthetic JSON samples define concrete values for the [API contract](../book/src/contract.md), which is maintained in Chinese only. The frontend uses them as mock data. Backend contract tests serialize matching DTOs and compare the complete JSON values, including field names, nulls and sample values, then verify deserialization.

For a wire-format change, update the chapter and samples in a separate `docs: update API contract for ...` commit before the implementation commits. The full workflow is in [CONTRIBUTING.md](../CONTRIBUTING.md).

Directories group related samples. The existing `m1/`, `m2/`, `m3/`, `m4/` and `m6/` names are retained fixture paths, not the current roadmap structure. Other groups are named for features, such as `reviews/`, `settings/`, `loudness/` and `webdav/`. Use synthetic values for every new sample.

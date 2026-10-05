# SDK lockstep publication

`nebula-sdk` is the supported Rust product. Its internal dependencies remain
technical implementation packages; publishing them does not make their direct
use a supported integration surface.

`cargo xtask packaging plan` derives the publication set from Cargo metadata.
Starting at the SDK, it follows every internal normal/build dependency,
including optional and target-specific declarations retained in normalized
archives. Development dependencies do not enlarge this product boundary.
Every selected package has the SDK version, permits publication, and gives
each internal path dependency an exact `=0.32.0` requirement. Packages outside
the set have `publish = false`. The command fails with the owning package and
dependency when a pin is absent or inexact.

`cargo xtask packaging verify --output <new-directory>` runs real Cargo package
verification and publish dry-run for the complete selected set, with all
features enabled and the lockfile locked. It never uploads a package. Pinned
Cargo supports multi-package verification through its temporary registry
overlay: unpublished internal archives are staged locally and built from
their normalized registry manifests. This verifies the lockstep release set;
it does not claim those dependencies already exist in the public registry.
A standalone SDK-only registry dry-run requires the internal versions to have
been published first.

Failures retain command logs and emit no successful observation. Success
records the exact selected commands, archive identities, byte counts and
SHA-256 digests, and normalized manifests extracted from the actual `.crate`
archives. The NS19 semantic predicate rederives the set at the verifying
revision and checks every retained internal declaration, preserving its
package, alias, dependency kind and target condition. After CP3 admits the
binary inventory through trusted runner provenance, the verifier independently
rehashes the actual archives and extracts their normalized manifests again.
Missing, extra, changed, linked or oversized archives fail verification.
A local packaging report alone does not activate the gate.

The protected runner invokes
`cargo xtask packaging verify-archives --report <artifact-root>/NS19/published-manifest-precision/published-manifest-precision.json --archive-root <artifact-root>/NS19/published-manifest-precision/archives`
after inventory admission. The report is bounded to 4 MiB; each compressed
archive to 64 MiB, each fully decoded stream to 256 MiB, each normalized
manifest to 1 MiB, and the binary archive set to 512 MiB. Decoding reads at
most the stream limit plus one byte and rejects excess data before parsing
the tar, including padding after an otherwise valid manifest.

# Commutative marker cache workload

Run `cargo test -p uv-pep508 --lib commutative_cache_workload` to exercise a deterministic
private-interner workload. Six atomic markers span version ranges, string equality, substring
membership, and extras. Two compound trees and each operand's complement produce 16 operands and 256
operand pairs.

Each conjunction and disjunction starts with an empty operation cache. Interned nodes remain
canonical. Reversing the operands must return the same node without adding any cache entries; De
Morgan identities and complementary operands provide separate semantic controls. The snapshot
records 240 cold cache entries for each operator, 32 trivial calls for each operator, and at most
two entries per cold call. All 512 reversed calls add zero entries.

These counts expose cache reuse across complemented and compound nodes. They do not estimate a
production hit rate, memory reduction, or end-to-end speedup; clearing the operation cache makes the
cold-call controls intentionally unlike a persistent resolver workload.

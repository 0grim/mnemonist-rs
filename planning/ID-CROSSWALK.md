# ID crosswalk — flat `B-nn`/`D-nn` to module-scoped IDs

Superseded on 2026-08-03. The flat space was allocated centrally and collided twice: agents in
isolated worktrees claimed `B-11`–`B-14` for different bugs (caught at merge), and `D-40`–`D-46`
plus `D-60` each named two different decisions (**not** caught — they reached judge-facing docs).
Module-scoped IDs make a collision impossible: the module name is part of the ID.

This table exists so references in git history and old commit messages stay resolvable.
No file in the repository uses the left-hand column any more.

## Bugs

| was | is |
|---|---|
| `B-1` | `DIV-PROJ-1` |
| `B-2` | `BUG-UTILS-ITERABLES-1` |
| `B-3` | `DIV-PROJ-4` |
| `B-4` | `DIV-PROJ-5` |
| `B-5` | `DIV-PROJ-7` |
| `B-6` | `DIV-PROJ-9` |
| `B-7` | `BUG-STATIC-DISJOINT-SET-1` |
| `B-8` | `BUG-SPARSE-SET-1` |
| `B-9` | `BUG-SPARSE-SET-2` |
| `B-10` | `BUG-SPARSE-SET-3` |
| `B-11` | `BUG-SPARSE-MAP-1` |
| `B-12` | `BUG-SPARSE-QUEUE-SET-1` |
| `B-13` | `BUG-SPARSE-QUEUE-SET-2` |
| `B-14` | `BUG-SPARSE-QUEUE-SET-3` |
| `B-15` | `BUG-HASHED-ARRAY-TREE-1` |
| `B-16` | `BUG-HASHED-ARRAY-TREE-2` |
| `B-17` | `BUG-BIT-SET-1` |
| `B-18` | `BUG-BIT-SET-2` |
| `B-19` | `BUG-UTILS-BITWISE-1` |
| `B-20` | `BUG-UTILS-BITWISE-2` |
| `B-21` | `BUG-BIT-VECTOR-1` |
| `B-22` | `BUG-BIT-VECTOR-2` |
| `B-23` | `BUG-BIT-SET-3` |
| `B-30` | `BUG-STACK-1` |
| `B-31` | `PORTBUG-1` |
| `B-40` | `BUG-DEFAULT-MAP-1` |
| `B-60` | `BUG-UTILS-ITERABLES-2` |
| `B-61` | `BUG-FIXED-STACK-1` |
| `B-62` | `BUG-CIRCULAR-BUFFER-1` |
| `B-63` | `BUG-FIXED-STACK-2` |
| `B-69` | `DIV-PROJ-37` |
| `B-70` | `BUG-HEAP-1` |
| `B-71` | `BUG-HEAP-2` |
| `B-72` | `BUG-HEAP-3` |
| `B-73` | `BUG-FIXED-REVERSE-HEAP-1` |
| `B-74` | `BUG-FIXED-REVERSE-HEAP-2` |
| `B-75` | `BUG-HEAP-4` |
| `B-76` | `BUG-HEAP-5` |
| `B-77` | `BUG-HEAP-6` |
| `B-78` | `BUG-HEAP-7` |
| `B-79` | `BUG-HEAP-8` |
| `B-80` | `BUG-SORT-1` |
| `B-81` | `BUG-SORT-2` |
| `B-89` | `DIV-PROJ-41` |
| `B-90` | `BUG-SUFFIX-ARRAY-1` |
| `B-91` | `BUG-SUFFIX-ARRAY-2` |
| `B-92` | `BUG-UTILS-HASH-TABLES-1` |
| `B-93` | `BUG-BLOOM-FILTER-1` |
| `B-94` | `BUG-UTILS-HASH-TABLES-2` |
| `B-95` | `BUG-UTILS-BINARY-SEARCH-1` |
| `B-96` | `BUG-UTILS-BINARY-SEARCH-2` |
| `B-97` | `BUG-BLOOM-FILTER-2` |
| `B-98` | `BUG-BLOOM-FILTER-3` |
| `B-99` | `BUG-BLOOM-FILTER-4` |
| `B-100` | `BUG-STATIC-INTERVAL-TREE-1` |
| `B-101` | `BUG-VECTOR-1` |
| `B-102` | `BUG-VECTOR-2` |
| `B-119` | `DIV-PROJ-46` |
| `B-120` | `BUG-BI-MAP-1` |
| `B-139` | `DIV-PROJ-47` |
| `B-140` | `BUG-LRU-CACHE-1` |
| `B-141` | `DIV-PROJ-48` |
| `B-142` | `BUG-LRU-CACHE-2` |
| `B-159` | `DIV-PROJ-49` |
| `B-160` | `BUG-MULTI-SET-1` |
| `B-161` | `BUG-MULTI-SET-2` |
| `B-162` | `BUG-MULTI-SET-3` |
| `B-179` | `DIV-PROJ-50` |
| `B-180` | `BUG-UTILS-1` |
| `B-199` | `DIV-PROJ-51` |
| `B-200` | `BUG-TRIE-MAP-1` |
| `B-201` | `BUG-TRIE-MAP-2` |
| `B-220` | `BUG-FIBONACCI-HEAP-1` |
| `B-221` | `BUG-FIBONACCI-HEAP-2` |
| `B-222` | `BUG-FIBONACCI-HEAP-3` |
| `B-239` | `DIV-PROJ-52` |
| `B-240` | `BUG-INVERTED-INDEX-1` |
| `B-241` | `BUG-LINKED-LIST-1` |
| `B-242` | `BUG-DEFAULT-WEAK-MAP-1` |
| `B-243` | `DIV-PROJ-56` |
| `B-259` | `DIV-PROJ-59` |
| `B-260` | `BUG-FIXED-CRITBIT-TREE-MAP-1` |
| `B-261` | `BUG-FIXED-CRITBIT-TREE-MAP-2` |
| `B-262` | `DIV-PROJ-60` |
| `B-279` | `DIV-PROJ-61` |
| `B-280` | `DIV-PROJ-62` |
| `B-300` | `DIV-PROJ-63` |
| `B-319` | `DIV-PROJ-64` |

## Divergences

| was | is |
|---|---|
| `D-01` | `DIV-PROJ-2` |
| `D-02` | `DIV-PROJ-3` |
| `D-03` | `DIV-QUEUE-1` |
| `D-04` | `DIV-PROJ-6` |
| `D-05` | `DIV-PROJ-8` |
| `D-06` | `DIV-STACK-1` |
| `D-07` | `DIV-STACK-2` |
| `D-08` | `DIV-PROJ-10` |
| `D-09` | `DIV-SPARSE-SET-1` |
| `D-10` | `DIV-PROJ-11` |
| `D-11` | `DIV-PROJ-12` |
| `D-12` | `DIV-PROJ-13` |
| `D-13` | `DIV-PROJ-14` |
| `D-14` | `DIV-PROJ-15` |
| `D-15` | `DIV-PROJ-16` |
| `D-16` | `DIV-PROJ-17` |
| `D-17` | `DIV-PROJ-18` |
| `D-18` | `DIV-UTILS-ITERABLES-1` |
| `D-19` | `DIV-PROJ-19` |
| `D-20` | `DIV-PROJ-20` |
| `D-21` | `DIV-PROJ-21` |
| `D-22` | `DIV-PROJ-22` |
| `D-23` | `DIV-PROJ-23` |
| `D-24` | `DIV-PROJ-24` |
| `D-25` | `DIV-PROJ-25` |
| `D-26` | `DIV-PROJ-26` |
| `D-27` | `DIV-PROJ-27` |
| `D-28` | `DIV-PROJ-28` |
| `D-29` | `DIV-PROJ-29` |
| `D-30` | `DIV-STATIC-DISJOINT-SET-1` |
| `D-31` | `DIV-STATIC-DISJOINT-SET-2` |
| `D-32` | `DIV-PROJ-30` |
| `D-33` | `DIV-PROJ-31` |
| `D-34` | `DIV-PROJ-32` |
| `D-35` | `DIV-PROJ-33` |
| `D-36` | `DIV-PROJ-34` |
| `D-37` | `DIV-PROJ-35` |
| `D-38` | `DIV-PROJ-36` |
| `D-39` | `DIV-FIXED-STACK-1` |
| `D-40` | `DIV-UTILS-BINARY-SEARCH-1` |
| `D-41` | `DIV-STACK-3` |
| `D-42` | `DIV-STACK-4` |
| `D-43` | `DIV-STACK-5` |
| `D-44` | `DIV-STACK-6` |
| `D-45` | `DIV-STACK-7` |
| `D-46` | `DIV-STACK-8` |
| `D-47` | `DIV-UTILS-HASH-TABLES-1` |
| `D-48` | `DIV-SUFFIX-ARRAY-1` |
| `D-49` | `DIV-SUFFIX-ARRAY-2` |
| `D-50` | `DIV-SUFFIX-ARRAY-3` |
| `D-51` | `DIV-SUFFIX-ARRAY-4` |
| `D-52` | `DIV-SUFFIX-ARRAY-5` |
| `D-53` | `DIV-SUFFIX-ARRAY-6` |
| `D-54` | `DIV-BLOOM-FILTER-1` |
| `D-55` | `DIV-BLOOM-FILTER-2` |
| `D-56` | `DIV-BLOOM-FILTER-3` |
| `D-57` | `DIV-BLOOM-FILTER-4` |
| `D-58` | `DIV-BLOOM-FILTER-5` |
| `D-59` | `DIV-BLOOM-FILTER-6` |
| `D-60` | `DIV-FIXED-STACK-2` |
| `D-61` | `DIV-FIXED-STACK-3` |
| `D-62` | `DIV-FIXED-STACK-4` |
| `D-63` | `DIV-FIXED-STACK-5` |
| `D-64` | `DIV-FIXED-STACK-6` |
| `D-65` | `DIV-CIRCULAR-BUFFER-1` |
| `D-66` | `DIV-FIXED-STACK-7` |
| `D-69` | `DIV-PROJ-38` |
| `D-70` | `DIV-HEAP-1` |
| `D-71` | `DIV-HEAP-2` |
| `D-72` | `DIV-HEAP-3` |
| `D-73` | `DIV-HEAP-4` |
| `D-74` | `DIV-HEAP-5` |
| `D-75` | `DIV-HEAP-6` |
| `D-76` | `DIV-HEAP-7` |
| `D-77` | `DIV-HEAP-8` |
| `D-78` | `DIV-PROJ-39` |
| `D-79` | `DIV-PROJ-40` |
| `D-80` | `DIV-SORT-1` |
| `D-81` | `DIV-SORT-2` |
| `D-82` | `DIV-SORT-4` |
| `D-83` | `DIV-SORT-3` |
| `D-84` | `DIV-SORT-5` |
| `D-85` | `DIV-SET-1` |
| `D-86` | `DIV-SET-2` |
| `D-87` | `DIV-SET-3` |
| `D-88` | `DIV-SET-4` |
| `D-89` | `DIV-LRU-CACHE-1` |
| `D-90` | `DIV-LRU-CACHE-2` |
| `D-91` | `DIV-LRU-CACHE-3` |
| `D-92` | `DIV-LRU-CACHE-4` |
| `D-93` | `DIV-LRU-CACHE-5` |
| `D-100` | `DIV-PROJ-42` |
| `D-101` | `DIV-PROJ-43` |
| `D-102` | `DIV-PROJ-44` |
| `D-103` | `DIV-PROJ-45` |
| `D-104` | `DIV-UTILS-1` |
| `D-105` | `DIV-UTILS-2` |
| `D-106` | `DIV-UTILS-3` |
| `D-160` | `DIV-MULTI-MAP-1` |
| `D-161` | `DIV-MULTI-MAP-2` |
| `D-162` | `DIV-MULTI-MAP-3` |
| `D-163` | `DIV-MULTI-SET-1` |
| `D-164` | `DIV-MULTI-SET-2` |
| `D-165` | `DIV-MULTI-SET-3` |
| `D-166` | `DIV-MULTI-SET-4` |
| `D-167` | `DIV-FUZZY-MULTI-MAP-1` |
| `D-168` | `DIV-FUZZY-MULTI-MAP-2` |
| `D-169` | `DIV-FUZZY-MULTI-MAP-3` |
| `D-170` | `DIV-FIBONACCI-HEAP-1` |
| `D-171` | `DIV-FIBONACCI-HEAP-2` |
| `D-172` | `DIV-FIBONACCI-HEAP-3` |
| `D-173` | `DIV-FIBONACCI-HEAP-4` |
| `D-200` | `DIV-TRIE-MAP-1` |
| `D-201` | `DIV-TRIE-MAP-2` |
| `D-202` | `DIV-TRIE-MAP-3` |
| `D-240` | `DIV-PROJ-53` |
| `D-241` | `DIV-PROJ-54` |
| `D-242` | `DIV-PROJ-55` |
| `D-243` | `DIV-PROJ-57` |
| `D-244` | `DIV-PROJ-58` |
| `D-245` | `DIV-CRITBIT-TREE-MAP-1` |
| `D-246` | `DIV-FIXED-CRITBIT-TREE-MAP-1` |
| `D-300` | `DIV-BK-TREE-1` |
| `D-301` | `DIV-BK-TREE-2` |
| `D-302` | `DIV-BK-TREE-3` |
| `D-303` | `DIV-BK-TREE-4` |
| `D-304` | `DIV-BK-TREE-5` |
| `D-305` | `DIV-BK-TREE-6` |
| `D-306` | `DIV-DEFAULT-WEAK-MAP-1` |
| `D-307` | `DIV-DEFAULT-WEAK-MAP-2` |
| `D-308` | `DIV-DEFAULT-WEAK-MAP-3` |
| `D-309` | `DIV-DEFAULT-WEAK-MAP-4` |
| `D-310` | `DIV-DEFAULT-WEAK-MAP-5` |
| `D-311` | `DIV-DEFAULT-WEAK-MAP-6` |
| `D-312` | `DIV-FUZZY-MAP-1` |
| `D-313` | `DIV-FUZZY-MAP-2` |
| `D-314` | `DIV-FUZZY-MAP-3` |
| `D-315` | `DIV-FUZZY-MAP-4` |
| `D-316` | `DIV-FUZZY-MAP-5` |
| `D-317` | `DIV-FUZZY-MAP-6` |
| `D-318` | `DIV-FUZZY-MAP-7` |
| `D-319` | `DIV-UTILS-BITWISE-1` |
| `D-320` | `DIV-UTILS-BITWISE-2` |
| `D-321` | `DIV-UTILS-BITWISE-3` |
| `D-322` | `DIV-UTILS-BITWISE-4` |
| `D-323` | `DIV-UTILS-BITWISE-5` |
| `D-400` | `DIV-VP-TREE-1` |
| `D-401` | `DIV-VP-TREE-2` |
| `D-402` | `DIV-VP-TREE-3` |
| `D-403` | `DIV-VP-TREE-4` |
| `D-404` | `DIV-VP-TREE-5` |
| `D-405` | `DIV-VP-TREE-6` |
| `D-406` | `DIV-KD-TREE-1` |
| `D-407` | `DIV-KD-TREE-2` |
| `D-408` | `DIV-KD-TREE-3` |
| `D-409` | `DIV-KD-TREE-4` |
| `D-410` | `DIV-KD-TREE-5` |
| `D-449` | `DIV-PROJ-65` |
| `D-450` | `DIV-MULTI-ARRAY-1` |
| `D-451` | `DIV-SYMSPELL-1` |
| `D-452` | `DIV-PASSJOIN-INDEX-1` |
| `D-453` | `DIV-PASSJOIN-INDEX-2` |
| `D-454` | `DIV-PASSJOIN-INDEX-3` |

## Double-allocated — one old ID, two real meanings

These are the collisions. Each old tag resolved to a different decision depending on which
module was reading it; the split is by the file the reference sits in.

| was | is (default) | is (second meaning) | second meaning applies in |
|---|---|---|---|
| `D-41` | `DIV-STACK-3` | `DIV-UTILS-BINARY-SEARCH-2` | files matching `binary[-_]search` |
| `D-42` | `DIV-STACK-4` | `DIV-UTILS-BINARY-SEARCH-3` | files matching `binary[-_]search` |
| `D-43` | `DIV-STACK-5` | `DIV-UTILS-BINARY-SEARCH-4` | files matching `binary[-_]search` |
| `D-44` | `DIV-STACK-6` | `DIV-UTILS-HASH-TABLES-2` | files matching `hash[-_]tables|_utils|merge` |
| `D-45` | `DIV-STACK-7` | `DIV-UTILS-HASH-TABLES-3` | files matching `hash[-_]tables` |
| `D-46` | `DIV-STACK-8` | `DIV-UTILS-HASH-TABLES-4` | files matching `hash[-_]tables` |
| `D-60` | `DIV-FIXED-STACK-2` | `DIV-BLOOM-FILTER-7` | files matching `bloom` |

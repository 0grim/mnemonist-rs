//! Port of upstream `bk-tree.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A Burkhard-Keller tree: every node is an item plus a table of children
//! keyed by their **distance** from that item, where distance comes from a
//! caller-supplied metric. `add` walks down by distance until it finds an
//! empty slot; `search` is a bounded DFS that only descends into a child
//! whose distance key falls in `[d - n, d + n]`.
//!
//! # Not a T3 module
//!
//! Nothing here is a `Map`. Upstream's `node.children` is a plain object keyed
//! by the numeric distance, read and written by direct property access and
//! walked by a bounded numeric range in `search` — never enumerated. A plain
//! `HashMap<i64, Node<I>>` reproduces exactly that: `add` does one `get`/
//! `insert` by an exact distance, and `search` probes `d - n ..= d + n` one
//! value at a time, which is precisely upstream's
//! `for (i = d - n, l = d + n + 1; i < l; i++)` loop. No ordering machinery is
//! needed because nothing here ever iterates the *keys* of a children table —
//! only individual, know-in-advance distances.
//!
//! # The real re-entrancy hazard: the distance function
//!
//! `distance` is a caller-supplied callback, called from deep inside both
//! `add`'s descent and `search`'s traversal — this is the bridge's
//! `RefCell`-and-`FnMut` problem (PORTBUG-1 and `crate::structures::bit_vector`'s
//! growth policy), not a `Map` problem, so this module inherits *that* half of
//! the T3 lesson and none of the `OrderedMap` half. Core expresses the
//! fallibility explicitly: [`BkTree::try_add`] and [`BkTree::try_search`] take
//! a `FnMut(&I, &I) -> Result<i64, E>`, so a JS distance function that throws
//! propagates as an `Err` and leaves the tree exactly as it was — upstream's
//! `this.size++`/`node.children[d] = …` lines are both *after* the call that
//! can throw, in both `add` and (trivially, since `search` never mutates)
//! `search`. [`BkTree::add`]/[`BkTree::search`] are the infallible
//! conveniences for a Rust caller whose distance function cannot fail, built
//! the same way [`crate::structures::default_map::DefaultMap::get_or_insert_with`]
//! is built over its fallible sibling.
//!
//! # Search order is part of the contract
//!
//! ```js
//! BKTree.prototype.search = function(n, query) {
//!   ...
//!   var found = [], stack = [this.root], node, child, d, i, l;
//!   while (stack.length) {
//!     node = stack.pop();
//!     d = this.distance(query, node.item);
//!     if (d <= n) found.push({item: node.item, distance: d});
//!     for (i = d - n, l = d + n + 1; i < l; i++) {
//!       child = node.children[i];
//!       if (child) stack.push(child);
//!     }
//!   }
//!   return found;
//! };
//! ```
//!
//! The inner loop pushes children in **ascending** distance order onto a
//! stack, which then pops them in **descending** order — so a node's
//! higher-distance children are explored before its lower-distance ones, and
//! the original test asserts on the resulting order
//! (`tree.search(2, 'mello')` returns `hello` before `yellow`, not the reverse).
//! [`BkTree::try_search`] reproduces the loop and the stack verbatim, in that
//! order, rather than collecting and sorting — a sorted result would agree on
//! *membership* and diverge on *order*, which is exactly the kind of "more
//! correct, and therefore wrong" divergence this port's bug-for-bug rule
//! exists to catch.
//!
//! # What this deliberately does not model
//!
//! A `distance` that returns something other than a finite integer — negative,
//! fractional, `NaN` — has no test anywhere in the upstream suite (every real
//! distance metric, `levenshtein` included, returns a non-negative integer).
//! Upstream would coerce such a value into an object-property key via
//! `ToPropertyKey`, which stringifies *anything*; reproducing that would need
//! a string-keyed children table and would buy nothing no test can observe.
//! The bridge therefore requires the JS distance function to return a real
//! number and this core takes `i64` outright, documented as a stated
//! narrowing rather than silently mismodelled.

use std::collections::HashMap;

/// One node: an item, and its children keyed by their distance from it.
#[derive(Debug, Clone)]
struct Node<I> {
    item: I,
    children: HashMap<i64, Node<I>>,
}

/// One `search` hit — upstream's `{item, distance}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found<I> {
    /// The stored item that matched.
    pub item: I,
    /// Its distance from the query, as reported by the caller-supplied
    /// distance function.
    pub distance: i64,
}

/// Upstream's `BKTree`.
///
/// `I` is the item type; `distance` is supplied per call, never stored — the
/// same reasoning as `DefaultMap`'s factory (`default_map.rs`): the
/// JS callback belongs at the boundary, and a Rust caller of this type never
/// needs to know a bridge exists.
#[derive(Debug, Clone)]
pub struct BkTree<I> {
    root: Option<Node<I>>,
    size: usize,
}

impl<I> Default for BkTree<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I> BkTree<I> {
    /// An empty tree — `new BKTree(distance)`, minus the distance function,
    /// which this port takes per call instead of storing.
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
        }
    }

    /// Upstream's `size` — the number of items added, including any that
    /// landed at a distance already occupied.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Upstream's `clear`.
    pub fn clear(&mut self) {
        self.root = None;
        self.size = 0;
    }
}

impl<I: Clone> BkTree<I> {
    /// Upstream's `add`, with a distance function that cannot fail.
    pub fn add<F>(&mut self, item: I, mut distance: F)
    where
        F: FnMut(&I, &I) -> i64,
    {
        match self.try_add(item, |a, b| {
            Ok::<i64, std::convert::Infallible>(distance(a, b))
        }) {
            Ok(()) => {}
            Err(never) => match never {},
        }
    }

    /// Upstream's `add`, for a distance function that can fail (the JS
    /// bridge's case: the callback can throw).
    ///
    /// A failing call leaves the tree **exactly as it was** — no node, no
    /// `size` increment — because both of upstream's mutations are textually
    /// after the `this.distance(...)` call that can throw, in every path
    /// through the loop.
    pub fn try_add<F, E>(&mut self, item: I, mut distance: F) -> Result<(), E>
    where
        F: FnMut(&I, &I) -> Result<i64, E>,
    {
        let Some(root) = &mut self.root else {
            self.root = Some(Node {
                item,
                children: HashMap::new(),
            });
            self.size += 1;

            return Ok(());
        };

        let mut node = root;

        loop {
            let d = distance(&item, &node.item)?;

            node = match node.children.entry(d) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(Node {
                        item,
                        children: HashMap::new(),
                    });
                    self.size += 1;

                    return Ok(());
                }
                std::collections::hash_map::Entry::Occupied(slot) => slot.into_mut(),
            };
        }
    }

    /// Upstream's `search`, with a distance function that cannot fail.
    pub fn search<F>(&self, n: i64, query: &I, mut distance: F) -> Vec<Found<I>>
    where
        F: FnMut(&I, &I) -> i64,
    {
        match self.try_search(n, query, |a, b| {
            Ok::<i64, std::convert::Infallible>(distance(a, b))
        }) {
            Ok(found) => found,
            Err(never) => match never {},
        }
    }

    /// Upstream's `search`, for a distance function that can fail.
    ///
    /// A failing call abandons the walk and returns `Err`, discarding
    /// whatever partial `found` had accumulated — exactly what upstream does:
    /// an exception thrown mid-`search` propagates out of the function, and
    /// the local `found` array is never returned to anyone.
    pub fn try_search<F, E>(&self, n: i64, query: &I, mut distance: F) -> Result<Vec<Found<I>>, E>
    where
        F: FnMut(&I, &I) -> Result<i64, E>,
    {
        let Some(root) = &self.root else {
            return Ok(Vec::new());
        };

        let mut found = Vec::new();
        let mut stack = vec![root];

        while let Some(node) = stack.pop() {
            let d = distance(query, &node.item)?;

            if d <= n {
                found.push(Found {
                    item: node.item.clone(),
                    distance: d,
                });
            }

            // Ascending push order onto a LIFO stack: see the module docs for
            // why this is part of the observable contract, not an
            // implementation detail free to change.
            let lo = d - n;
            let hi = d + n;
            let mut i = lo;

            while i <= hi {
                if let Some(child) = node.children.get(&i) {
                    stack.push(child);
                }

                i += 1;
            }
        }

        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upstream suite's distance: Levenshtein over short lowercase words,
    /// small enough to hand-verify. Ported directly rather than pulled in as a
    /// dependency — `mnemonist-core` is zero-dependency by declaration, and
    /// this is exactly the kind of small, fully-specified function that is
    /// cheaper to port than to justify a crate for.
    fn levenshtein(a: &str, b: &str) -> i64 {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<i64> = (0..=b.len() as i64).collect();

        for (i, &ca) in a.iter().enumerate() {
            let mut previous = row[0];
            row[0] = i as i64 + 1;

            for (j, &cb) in b.iter().enumerate() {
                let temp = row[j + 1];
                row[j + 1] = if ca == cb {
                    previous
                } else {
                    1 + previous.min(row[j]).min(row[j + 1])
                };
                previous = temp;
            }
        }

        row[b.len()]
    }

    /// `BkTree<&'static str>`'s distance function, as a real `fn` item rather
    /// than a closure: [`BkTree::add`]/[`BkTree::search`] take `F: FnMut(&I,
    /// &I) -> i64`, and `I = &'static str` makes that `FnMut(&&str, &&str)`,
    /// one reference deeper than `levenshtein`'s own signature. A bare
    /// function item's type is exactly its declared signature — there is no
    /// auto-deref for trait matching the way there is for a call — so this
    /// one extra `&` is spelled out once here rather than at every call site.
    fn dist(a: &&str, b: &&str) -> i64 {
        levenshtein(a, b)
    }

    fn tree() -> BkTree<&'static str> {
        BkTree::new()
    }

    /// 1:1 port of the upstream suite's five `it` blocks.
    #[test]
    fn reproduces_the_upstream_suite() {
        // …add…
        let mut counted = tree();
        counted.add("hello", dist);
        counted.add("roman", dist);
        counted.add("yellow", dist);
        assert_eq!(counted.size(), 3);

        // …clear…
        let mut cleared = counted.clone();
        cleared.clear();
        assert_eq!(cleared.size(), 0);

        // …search…
        let mut searched = tree();
        searched.add("hello", dist);
        searched.add("roman", dist);
        searched.add("yellow", dist);

        assert_eq!(
            searched.search(1, &"mello", dist),
            vec![Found {
                item: "hello",
                distance: 1
            }]
        );
        assert_eq!(
            searched.search(2, &"mello", dist),
            vec![
                Found {
                    item: "hello",
                    distance: 1
                },
                Found {
                    item: "yellow",
                    distance: 2
                },
            ]
        );

        // …arbitrary objects: the tree never inspects `item`, so a tuple
        // stands in for upstream's `{value: "..."}`…
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct Item(&'static str);

        let mut objects: BkTree<Item> = BkTree::new();
        objects.add(Item("hello"), |a, b| levenshtein(a.0, b.0));
        objects.add(Item("roman"), |a, b| levenshtein(a.0, b.0));
        objects.add(Item("yellow"), |a, b| levenshtein(a.0, b.0));

        assert_eq!(
            objects.search(1, &Item("mello"), |a, b| levenshtein(a.0, b.0)),
            vec![Found {
                item: Item("hello"),
                distance: 1
            }]
        );

        // …from (mirrored: add in iteration order)…
        let mut from_iter = tree();
        for word in ["hello", "yellow"] {
            from_iter.add(word, dist);
        }
        assert_eq!(from_iter.size(), 2);
    }

    #[test]
    fn a_search_with_no_root_returns_nothing() {
        let empty: BkTree<&str> = BkTree::new();

        assert_eq!(empty.search(5, &"anything", dist), vec![]);
    }

    /// Same node set, but confirming the DESCENDING order the LIFO stack
    /// produces from an ASCENDING push loop — see the module docs.
    #[test]
    fn search_visits_higher_distance_children_before_lower_distance_ones() {
        let mut t = tree();
        // Every word is built to sit at a distinct, known distance from the
        // root so the traversal order is unambiguous.
        t.add("aaaa", dist); // root
        t.add("baaa", dist); // distance 1 from root
        t.add("bbaa", dist); // distance 2 from root
        t.add("bbba", dist); // distance 3 from root

        let hits = t.search(3, &"aaaa", dist);
        let order: Vec<&str> = hits.iter().map(|found| found.item).collect();

        // The root (distance 0) is visited first because it starts the
        // stack; its three children are then popped highest-distance-first.
        assert_eq!(order, vec!["aaaa", "bbba", "bbaa", "baaa"]);
    }

    #[test]
    fn a_failing_distance_leaves_add_with_no_trace() {
        let mut t = tree();
        t.add("hello", dist);

        let outcome: Result<(), &str> = t.try_add("world", |_, _| Err("boom"));

        assert_eq!(outcome, Err("boom"));
        assert_eq!(t.size(), 1, "the failed add must not be counted");
        assert_eq!(
            t.search(0, &"hello", dist),
            vec![Found {
                item: "hello",
                distance: 0
            }],
            "the tree is otherwise untouched"
        );
    }

    #[test]
    fn a_failing_distance_during_descent_leaves_no_trace_either() {
        let mut t = tree();
        t.add("hello", dist);
        t.add("mello", dist); // distance 1 from "hello"

        // Fails only on the second distance call, i.e. once the walk has
        // already descended past the root.
        let mut calls = 0;
        let outcome: Result<(), &str> = t.try_add("wello", |a, b| {
            calls += 1;
            if calls == 1 {
                Ok(levenshtein(a, b))
            } else {
                Err("boom")
            }
        });

        assert_eq!(outcome, Err("boom"));
        assert_eq!(t.size(), 2);
    }

    #[test]
    fn a_failing_distance_during_search_discards_the_partial_result() {
        let mut t = tree();
        t.add("hello", dist);
        t.add("world", dist);

        let mut calls = 0;
        let outcome: Result<Vec<Found<&str>>, &str> = t.try_search(10, &"hullo", |a, b| {
            calls += 1;
            if calls == 1 {
                Ok(levenshtein(a, b))
            } else {
                Err("boom")
            }
        });

        assert_eq!(outcome, Err("boom"));
    }

    #[test]
    fn size_counts_only_successful_inserts() {
        let mut t = tree();
        t.add("a", dist);
        t.add("a", dist); // same distance-0 slot exists; upstream still walks and inserts under whatever distance is free
        assert_eq!(
            t.size(),
            2,
            "add never checks for duplicates, upstream included"
        );
    }

    #[test]
    fn clear_resets_size_and_forgets_every_node() {
        let mut t = tree();
        t.add("hello", dist);
        t.add("world", dist);

        t.clear();

        assert_eq!(t.size(), 0);
        assert_eq!(t.search(100, &"hello", dist), vec![]);
    }
}

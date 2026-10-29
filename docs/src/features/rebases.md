# Rebases

REBASE (`g r`) lists the rebases in git's reflog, newest first, and shows what was decided by hand
in each.

- For each pick that stopped on a conflict, tuitor redoes git's merge (`git merge-file`) and shows
  where the rebased commit differs: which side a conflict kept, or what was merged by hand.
- It lists lines the branch had before the rebase and lost after it.
- It marks picks the target already had (rewritten by another rebase) and suggests the
  `git rebase --onto` that would have skipped them.

| key | does |
|---|---|
| `enter` | read a rebase; on a decision, open its line in CODE |
| `h` `l` | fold, unfold |
| `b` | a breakpoint set of a rebase's, a pick's or one decision's lines |

The set records without stopping. Run the code, then use HISTORY and [COMPARE](../sets.md#compare)
to see what the decided code did.

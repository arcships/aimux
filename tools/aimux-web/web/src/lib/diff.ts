// Minimal line diff (LCS-based, size-capped) for the Replay comparison pane.

export interface DiffOp {
  type: 'same' | 'add' | 'del'
  text: string
}

const MAX_CELLS = 4_000_000

export function diffLines(a: string, b: string): DiffOp[] {
  const la = a.split('\n')
  const lb = b.split('\n')
  const n = la.length
  const m = lb.length
  if (n === 0 && m === 0) return []
  if (n * m > MAX_CELLS) {
    // Too large for DP — degrade to a coarse "changed" diff.
    const ops: DiffOp[] = []
    for (const t of la) ops.push({ type: 'del', text: t })
    for (const t of lb) ops.push({ type: 'add', text: t })
    return ops
  }

  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] =
        la[i] === lb[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1])
    }
  }

  const ops: DiffOp[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (la[i] === lb[j]) {
      ops.push({ type: 'same', text: la[i] })
      i++
      j++
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      ops.push({ type: 'del', text: la[i] })
      i++
    } else {
      ops.push({ type: 'add', text: lb[j] })
      j++
    }
  }
  while (i < n) ops.push({ type: 'del', text: la[i++] })
  while (j < m) ops.push({ type: 'add', text: lb[j++] })
  return ops
}

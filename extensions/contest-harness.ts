// Autocontest-Harness extension for omp and pi (one file; both expose the same extension API).
//
// Commands: /contest (board + capacity summary), /contest <id> (one entry), /contest-chat
// (how to open the main interface). All of them call the `harness` binary.
// pi only: the `contest_agent` tool runs an agents/<name>.md prompt in a one-shot pi, since
// pi has no subagents (omp has its own `task` tool). The persona and main interface role
// come from `harness chat` (--append-system-prompt), not from this extension.
//
// No package-specific type import: omp and pi publish their types under different names.
// Failures are reported to the UI or the tool result and never block the session.

import { execFile, spawn } from 'node:child_process'
import { existsSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

type NotifyType = 'info' | 'warning' | 'error'
interface HostUI {
  notify?(message: string, type?: NotifyType): void
}
interface HostContext {
  cwd?: unknown
  ui?: HostUI
}
interface ToolResult {
  content: Array<{ type: 'text'; text: string }>
  details: unknown
}
interface ToolDefinition {
  name: string
  label: string
  description: string
  promptSnippet?: string
  parameters: Record<string, unknown>
  execute(
    toolCallId: string,
    params: Record<string, unknown>,
    signal: AbortSignal | undefined,
    onUpdate: unknown,
    ctx: HostContext,
  ): Promise<ToolResult>
}
interface HostApi {
  registerCommand(name: string, options: { description?: string; handler: (args: string, ctx: HostContext) => Promise<void> }): void
  registerTool?(tool: ToolDefinition): void
}

// Display-only views of `harness --json` output; the binary owns these shapes.
interface Entry {
  id?: string
  title?: string
  state?: string
  level?: number | null
  type?: string | null
  priority?: number | null
  organizer?: string | null
  deadline?: string | null
  url?: string | null
  folder?: string | null
  repo?: string | null
  session?: { runner?: string; workspace?: string; handle?: string | null } | null
  gate?: { questions?: unknown } | null
  history?: Array<{ at?: string; from?: string; to?: string; note?: string | null }>
}
interface Capacity {
  slots?: number
  slots_by_level?: Record<string, number>
  recommended_workers?: number
  running?: number
  ram?: { budget_gb?: number; pressure?: string }
  limits?: { basis?: string }
}

// Why: the OMP binary loads the same file as pi; tell them apart by executable name.
const OMP_RUNTIME = [process.title, process.env._, process.argv[1], process.argv[0]]
  .map((v) => String(v || '').split(/[\\/]/).pop()?.toLowerCase() || '')
  .some((n) => ['omp', 'omp.js', 'omp.sh', 'omp.cmd', 'omp.exe', 'omp.bat'].includes(n))

// Why: `omp plugin link` / `pi install` load the file through a symlink; resolve to the checkout.
const ROOT = (() => {
  const here = typeof __dirname === 'string' ? __dirname : dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '..')
  try {
    return realpathSync(root)
  } catch {
    return root
  }
})()

const MAX_TOOL_CHARS = 50_000

function harnessBin(): string {
  const candidates = [process.env.HARNESS_BIN, join(ROOT, 'target', 'release', 'harness'), join(ROOT, 'target', 'debug', 'harness')]
  for (const c of candidates) if (c && existsSync(c)) return c
  return 'harness'
}

function errorText(err: unknown): string {
  const e = err as { stderr?: unknown; message?: unknown; code?: unknown }
  const stderr = typeof e?.stderr === 'string' ? e.stderr.trim() : ''
  if (stderr) return stderr.split('\n').slice(-5).join('\n')
  if (e?.code === 'ENOENT') return '`harness` 실행 파일을 찾지 못했다(`cargo build --release` 또는 HARNESS_BIN 설정 필요)'
  return String(e?.message ?? err)
}

function execHarness(args: string[]): Promise<string> {
  const { promise, resolve: done, reject: fail } = Promise.withResolvers<string>()
  execFile(
    harnessBin(),
    args,
    { cwd: ROOT, env: { ...process.env, HARNESS_HOME: process.env.HARNESS_HOME || ROOT }, maxBuffer: 32 * 1024 * 1024, timeout: 120_000 },
    (err, stdout, stderr) => (err ? fail(Object.assign(err, { stderr })) : done(stdout)),
  )
  return promise
}

async function runHarness(args: string[]): Promise<unknown> {
  const stdout = await execHarness([...args, '--json'])
  try {
    return JSON.parse(stdout)
  } catch {
    throw new Error(`harness ${args.join(' ')}: JSON이 아닌 출력`)
  }
}

function show(ctx: HostContext, text: string, type: NotifyType = 'info'): void {
  try {
    ctx.ui?.notify?.(text, type)
  } catch {
    // UI delivery must never fail the command.
  }
}

function daysLeft(deadline: unknown): number | null {
  if (typeof deadline !== 'string' || !deadline) return null
  const t = Date.parse(deadline.length === 10 ? `${deadline}T23:59:00+09:00` : deadline)
  return Number.isNaN(t) ? null : Math.floor((t - Date.now()) / 86_400_000)
}

function line(e: Entry): string {
  const d = daysLeft(e.deadline)
  const due = e.deadline ? ` · 마감 ${e.deadline}${d === null ? '' : d >= 0 ? ` (D-${d})` : ' (지남)'}` : ''
  const level = e.level ? ` · L${e.level}` : ''
  return `- ${e.id} ${e.title ?? ''} [${e.state}${level}]${due}`
}

function lastNote(e: Entry): string {
  const history = Array.isArray(e.history) ? e.history : []
  for (let i = history.length - 1; i >= 0; i--) {
    const note = history[i]?.note
    if (typeof note === 'string' && note) return note.length > 120 ? `${note.slice(0, 120)}…` : note
  }
  return ''
}

const IN_PROGRESS: Record<string, true> = { gate_pass: true, routed: true, running: true }

function boardSummary(entries: Entry[], cap: Capacity | null, boardPath: unknown): string {
  const out: string[] = []
  const counts = new Map<string, number>()
  for (const e of entries) counts.set(String(e.state), (counts.get(String(e.state)) ?? 0) + 1)
  const byState = [...counts].map(([s, n]) => `${s} ${n}`).join(' · ')
  out.push(`공모전 ${entries.length}건${byState ? `: ${byState}` : ''}`)

  const section = (title: string, rows: Entry[], note = false) => {
    if (!rows.length) return
    out.push(`${title} (${rows.length})`)
    for (const e of rows.slice(0, 10)) {
      const n = note ? lastNote(e) : ''
      out.push(`${line(e)}${n ? ` — ${n}` : ''}`)
    }
    if (rows.length > 10) out.push(`- … ${rows.length - 10}건 더`)
  }
  const byDeadline = (a: Entry, b: Entry) => String(a.deadline ?? '9999').localeCompare(String(b.deadline ?? '9999'))
  section('제출 승인 대기', entries.filter((e) => e.state === 'awaiting_approval').sort(byDeadline), true)
  section('계획 검수 대기(레벨 5)', entries.filter((e) => e.state === 'plan_review').sort(byDeadline), true)
  const holds = entries.filter((e) => e.state === 'hold').sort(byDeadline)
  if (holds.length) {
    out.push(`보류 (${holds.length})`)
    for (const e of holds.slice(0, 10)) {
      out.push(line(e))
      const qs = Array.isArray(e.gate?.questions) ? e.gate.questions : []
      for (const q of qs.slice(0, 3)) out.push(`    ? ${typeof q === 'string' ? q : JSON.stringify(q)}`)
    }
  }
  section(
    '마감 7일 안(진행 중)',
    entries
      .filter((e) => IN_PROGRESS[String(e.state)])
      .filter((e) => {
        const d = daysLeft(e.deadline)
        return d !== null && d >= 0 && d <= 7
      })
      .sort(byDeadline),
  )

  if (cap) {
    const slots = cap.slots_by_level && typeof cap.slots_by_level === 'object'
      ? Object.entries(cap.slots_by_level).map(([l, n]) => `L${l} ${n}`).join(' · ')
      : ''
    const ram = cap.ram ?? {}
    out.push(
      `용량: 새 세션 ${cap.slots ?? '?'}개${slots ? `(${slots})` : ''}, 권장 워커 ${cap.recommended_workers ?? '?'}명, ` +
        `진행 중 ${cap.running ?? '?'}, 메모리 예산 ${ram.budget_gb ?? '?'}GB·압박 ${ram.pressure ?? '?'}, 한도 기준 ${cap.limits?.basis ?? '?'}`,
    )
  }
  if (typeof boardPath === 'string') out.push(`보드: ${boardPath}`)
  return out.join('\n')
}

function entrySummary(e: Entry): string {
  const out = [line(e)]
  if (e.organizer) out.push(`주최: ${e.organizer}`)
  if (e.type) out.push(`유형: ${e.type}${e.priority != null ? ` · 우선순위 ${e.priority}` : ''}`)
  if (e.url) out.push(`공고: ${e.url}`)
  if (e.folder) out.push(`폴더: ${e.folder}`)
  if (e.repo) out.push(`저장소: ${e.repo}`)
  const s = e.session
  if (s && typeof s === 'object') {
    out.push(`세션: ${s.runner ?? '?'} @ ${s.workspace ?? '?'}${s.handle ? ` (${s.handle})` : ''}`)
  }
  const qs = Array.isArray(e.gate?.questions) ? e.gate.questions : []
  for (const q of qs.slice(0, 3)) out.push(`질문: ${typeof q === 'string' ? q : JSON.stringify(q)}`)
  const history = Array.isArray(e.history) ? e.history.slice(-3) : []
  for (const h of history) out.push(`기록: ${h.at ?? ''} ${h.from ? `${h.from}→` : ''}${h.to ?? ''}${h.note ? ` ${h.note}` : ''}`.trimEnd())
  return out.join('\n')
}

function stripFrontmatter(text: string): string {
  const t = text.replace(/^\uFEFF/, '').replace(/\r\n/g, '\n')
  if (!t.startsWith('---\n')) return t.trim()
  const end = t.indexOf('\n---', 4)
  if (end === -1) return t.trim()
  const after = t.indexOf('\n', end + 4)
  return (after === -1 ? '' : t.slice(after + 1)).trim()
}

function agentNames(): string[] {
  try {
    return readdirSync(join(ROOT, 'agents'))
      .filter((f) => f.endsWith('.md'))
      .map((f) => f.slice(0, -3))
      .sort()
  } catch {
    return []
  }
}


function runAgent(agent: string, task: string, cwd: string, signal: AbortSignal | undefined): Promise<string> {
  const file = join(ROOT, 'agents', `${agent}.md`)
  if (!/^[A-Za-z0-9_-]+$/.test(agent) || !existsSync(file)) {
    return Promise.reject(new Error(`알 수 없는 에이전트: ${agent} (가능: ${agentNames().join(', ')})`))
  }
  const dir = mkdtempSync(join(tmpdir(), 'contest-agent-'))
  const prompt = join(dir, `${agent}.md`)
  writeFileSync(prompt, `${stripFrontmatter(readFileSync(file, 'utf8'))}\n`)
  // Why: the child must not spawn further contest agents.
  const env = { ...process.env, CONTEST_HARNESS_AGENT: '1' }
  const { promise, resolve: done, reject: fail } = Promise.withResolvers<string>()
  const child = spawn('pi', ['-p', '--no-session', '--append-system-prompt', prompt, `Task: ${task}`], {
    cwd,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let stdout = ''
  let stderr = ''
  child.stdout.on('data', (d) => (stdout += d))
  child.stderr.on('data', (d) => (stderr = (stderr + d).slice(-8000)))
  const abort = () => child.kill('SIGTERM')
  signal?.addEventListener('abort', abort, { once: true })
  child.on('error', fail)
  child.on('close', (code, sig) => {
    signal?.removeEventListener('abort', abort)
    if (code === 0) done(stdout)
    else fail(new Error(`pi ${agent} 실패(${sig ?? `exit ${code}`}): ${stderr.trim().split('\n').slice(-10).join('\n')}`))
  })
  return promise.finally(() => rmSync(dir, { recursive: true, force: true }))
}

export default function (pi: HostApi): void {
  pi.registerCommand('contest', {
    description: '공모전 하네스: 인자 없으면 보드·용량 요약, <id>면 그 공모전',
    handler: async (args, ctx) => {
      const id = args.trim()
      try {
        if (id) {
          show(ctx, entrySummary((await runHarness(['show', id])) as Entry))
          return
        }
        const [board, list, cap] = await Promise.allSettled([runHarness(['board']), runHarness(['list']), runHarness(['capacity'])])
        if (list.status === 'rejected') throw list.reason
        const entries: Entry[] = Array.isArray(list.value) ? list.value : []
        const boardValue = board.status === 'fulfilled' ? board.value : null
        const text = boardSummary(
          entries,
          cap.status === 'fulfilled' ? (cap.value as Capacity) : null,
          boardValue && typeof boardValue === 'object' && 'board' in boardValue ? boardValue.board : null,
        )
        const errors = [board, cap].filter((r) => r.status === 'rejected').map((r) => errorText((r as PromiseRejectedResult).reason))
        show(ctx, errors.length ? `${text}\n오류: ${errors.join(' / ')}` : text, errors.length ? 'warning' : 'info')
      } catch (err) {
        show(ctx, `harness 실패: ${errorText(err)}`, 'error')
      }
    },
  })

  pi.registerCommand('contest-chat', {
    description: '공모전 하네스 메인 인터페이스(페르소나 + 지휘 역할) 여는 법',
    handler: async (_args, ctx) => {
      const { promise, resolve: done } = Promise.withResolvers<string>()
      execFile(harnessBin(), ['--version'], { cwd: ROOT, timeout: 10_000 }, (err, stdout) =>
        done(err ? `harness를 찾지 못했다: ${errorText(err)}` : stdout.trim()),
      )
      const version = await promise
      show(
        ctx,
        `메인 인터페이스는 터미널에서 연다:\n  cd ${ROOT} && harness chat [--runner omp|pi|claude|codex|dsh] [--workspace terminal|orca|tmux]\n` +
          `설정 러너(data/config.json runner)로 페르소나와 harness/prompts/main.md를 시스템 프롬프트에 덧붙여 저장소 루트에서 연다.\n${version}`,
      )
    },
  })

  if (!OMP_RUNTIME && pi.registerTool && process.env.CONTEST_HARNESS_AGENT !== '1') {
    const names = agentNames()
    pi.registerTool({
      name: 'contest_agent',
      label: 'Contest agent',
      description:
        `Run one Autocontest-Harness agent (agents/<agent>.md) as a one-shot pi subprocess in the current directory and return its final output. ` +
        `Use it where the harness instructions say to run an agent as a task (ContestGate, ContestJudge, ...). Agents: ${names.join(', ')}.`,
      promptSnippet: 'contest_agent: run a contest harness agent (ContestGate, ContestJudge, ...) as a subtask',
      parameters: {
        type: 'object',
        properties: {
          agent: names.length ? { type: 'string', enum: names, description: 'Agent name' } : { type: 'string', description: 'Agent name' },
          task: { type: 'string', description: 'Self-contained task for the agent' },
        },
        required: ['agent', 'task'],
        additionalProperties: false,
      },
      async execute(_id, params, signal, _onUpdate, ctx) {
        const agent = String(params.agent ?? '')
        const cwd = typeof ctx?.cwd === 'string' ? ctx.cwd : process.cwd()
        let text = (await runAgent(agent, String(params.task ?? ''), cwd, signal)).trim()
        if (text.length > MAX_TOOL_CHARS) text = `${text.slice(0, MAX_TOOL_CHARS)}\n[출력 ${text.length}자 중 앞 ${MAX_TOOL_CHARS}자만 표시]`
        return { content: [{ type: 'text', text: text || '(출력 없음)' }], details: { agent } }
      },
    })
  }
}

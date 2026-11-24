"""`controllers.writing` against recorded-shape runner output. No model, credential or registry write."""
import json
import subprocess
import tempfile
import unittest
import uuid
from pathlib import Path

from controllers import writing

OMP_MODEL = 'fixture/claude-opus'
AGY_MODEL = 'agy/gemini-4-hydrogen-high'


def omp_stream(model=OMP_MODEL, text='draft candidate', stop='stop', ended=True, content=None):
    provider, model_id = model.split('/', 1)
    items = [{'type': 'session', 'id': 'fixture'},
             {'type': 'message_end', 'message': {'role': 'user', 'content': [{'type': 'text', 'text': 'prompt'}]}},
             {'type': 'message_end', 'message': {'role': 'assistant', 'provider': provider, 'model': model_id,
              'stopReason': stop, 'content': content or [{'type': 'text', 'text': text}]}}]
    if ended:
        items.append({'type': 'agent_end', 'messages': []})
    return '\n'.join(json.dumps(item) for item in items) + '\n'


def agy_stream(model='gemini-4-hydrogen-high', text='polished candidate\n', status='SUCCESS', steps=('agent_response',),
               turns=1, **extra):
    items = [{'event': 'init', 'conversation_id': 'c1', 'init': {'model': model, 'tools': ['run_command']}},
             {'event': 'step_update', 'step_update': {'conversation_id': 'c1', 'step_type': 'user_input'}}]
    items += [{'event': 'step_update', 'step_update': {'conversation_id': 'c1', 'step_type': step}} for step in steps]
    items.append({'event': 'result', 'result': dict({'conversation_id': 'c1', 'status': status, 'response': text,
                                                    'num_turns': turns}, **extra)})
    return '\n'.join(json.dumps(item) for item in items) + '\n'


class WritingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name).resolve()
        self.source = self.folder / 'baseline.md'
        self.source.write_text('baseline 123\n')
        (self.folder / 'instructions.md').write_text('Keep every fact and number.\n')
        self.request = {'stage': 'edit', 'model': AGY_MODEL, 'source': 'baseline.md',
                        'instructions': 'instructions.md', 'run_id': str(uuid.uuid4())}
        self.entry = {'id': 'fixture-contest', 'state': 'running', 'folder': str(self.folder), 'submissions': [],
                      'gate': {'writing_models': {'draft': ['fixture/claude-*', 'fixture/*-astra'],
                                                  'edit': ['agy/gemini-*-hydrogen-*']}}}
        self.calls = []
        owner = self

        class Ach:
            def json(self, command, *args, **kwargs):
                owner.calls.append(command)
                if command == 'contract':
                    return {'protocol': 2}
                return {'revision': 3} if command == 'approval-status' else owner.entry
        self.ach = Ach()

    def execute(self, stdout=None, stderr='', code=0):
        calls = []
        agy = self.request['model'].startswith('agy/')

        def runner(argv, **kwargs):
            calls.append(argv)
            self.assertNotIn('shell', kwargs)
            directory = Path(kwargs['cwd'])
            prompt = (directory / 'prompt.md').read_text()
            if agy:
                self.assertEqual(argv[:3], ['agy', '--model', 'gemini-4-hydrogen-high'])
                self.assertEqual(json.loads(Path(argv[argv.index('--settings') + 1]).read_text()), writing.AGY_POLICY)
                # The prompt travels on stdin, never in argv.
                self.assertEqual(json.loads(kwargs['input']), {'event': 'user', 'message': {'content': prompt}})
                self.assertFalse(any('baseline 123' in part for part in argv))
            else:
                self.assertEqual(argv[0], 'omp')
                for flag in ('--no-tools', '--no-extensions', '--no-prewalk', '--no-session', '--no-context-files'):
                    self.assertIn(flag, argv)
                self.assertEqual(json.loads(Path(argv[argv.index('--config') + 1]).read_text()), writing.OMP_POLICY)
                self.assertEqual(Path(argv[argv.index('-e') + 1]).read_text(), writing.OMP_GUARD)
                self.assertEqual(kwargs['stdin'], subprocess.DEVNULL)
                self.assertEqual(argv[-1], '@' + str(directory / 'prompt.md'))
            default = agy_stream() if agy else omp_stream()
            return subprocess.CompletedProcess(argv, code, stdout=default if stdout is None else stdout, stderr=stderr)
        before = (self.source.read_bytes(), self.source.stat().st_mtime_ns)
        result = writing.run(self.ach, 'fixture-contest', self.request, runner)
        self.assertEqual(before, (self.source.read_bytes(), self.source.stat().st_mtime_ns))
        self.assertEqual(len(calls), 1)
        self.assertEqual(self.calls, ['contract', 'show', 'approval-status'])
        return result, self.folder / '.private/writing' / self.request['run_id']

    def draft(self):
        self.request.update(stage='draft', model=OMP_MODEL, run_id=str(uuid.uuid4()))
        self.calls.clear()

    def test_an_edit_on_agy_is_bound_to_the_session_model_inputs_and_candidate(self):
        result, directory = self.execute()
        self.assertEqual((result['code'], result['runner']), ('candidate_generated', 'agy'))
        self.assertEqual(result['observed_models'], [AGY_MODEL])
        self.assertEqual((result['contest_id'], result['revision']), ('fixture-contest', 3))
        self.assertTrue(result['review_required'])
        self.assertFalse(result['adopted'])
        report = writing.check(directory)
        self.assertTrue(report['verified'])
        names = {item['path'].rsplit('/', 1)[1] for item in report['evidence']}
        self.assertEqual(names, {'receipt.json', 'input.md', 'prompt.md', 'writing-models.json', 'runner-policy.json',
                                 'stdout.jsonl', 'stderr.log', 'candidate.md'})
        self.assertTrue(all(item['path'].startswith('.private/writing/') for item in report['evidence']))
        (directory / 'candidate.md').write_text('changed after the model answered')
        self.assertFalse(writing.check(directory)['verified'])

    def test_a_draft_on_omp_is_bound_to_the_answering_model(self):
        self.draft()
        result, directory = self.execute()
        self.assertEqual((result['code'], result['runner']), ('candidate_generated', 'omp'))
        self.assertEqual(result['observed_models'], [OMP_MODEL])
        report = writing.check(directory)
        self.assertTrue(report['verified'])
        self.assertIn('.private/writing/%s/runner-guard.ts' % self.request['run_id'],
                      [item['path'] for item in report['evidence']])
        self.assertEqual((directory / 'candidate.md').read_text(), 'draft candidate')

    def test_roles_follow_the_gate_patterns_and_need_an_exact_selector(self):
        models = self.entry['gate']['writing_models']
        for model in ('fixture/claude-opus-5-5', 'fixture/gpt-6-astra'):
            self.assertTrue(writing.model_allowed('draft', model, models['draft']))
            self.assertFalse(writing.model_allowed('edit', model, models['edit']))
        self.assertTrue(writing.model_allowed('edit', AGY_MODEL, models['edit']))
        for model in (AGY_MODEL, 'fixture/gpt-6-sol', '@task', 'fixture/*', 'claude-opus'):
            self.assertFalse(writing.model_allowed('draft', model, models['draft']))
        self.assertFalse(writing.model_allowed('edit', AGY_MODEL), 'no policy allows no model')
        self.assertEqual((writing.runner_of(AGY_MODEL), writing.runner_of(OMP_MODEL)), ('agy', 'omp'))

    def test_a_contest_without_the_policy_or_another_model_starts_nothing(self):
        for change in ({'gate': {}}, {'gate': {'writing_models': {'edit': ['agy/other-*']}}}):
            self.entry.update(change)
            with self.assertRaisesRegex(ValueError, 'writing_model_not_allowed'):
                writing.run(self.ach, 'fixture-contest', self.request, lambda *a, **k: self.fail('executed'))
        self.assertFalse((self.folder / '.private').exists())

    def test_another_model_or_a_claim_in_the_text_is_not_the_requested_model(self):
        result, directory = self.execute(agy_stream(model='gemini-4-carbon-high', text='Hydrogen wrote this'))
        self.assertEqual(result['code'], 'unexpected_model')
        self.assertEqual(result['observed_models'], ['agy/gemini-4-carbon-high'])
        self.assertFalse((directory / 'candidate.md').exists())
        self.assertFalse(writing.check(directory)['verified'])
        self.draft()
        result, directory = self.execute(omp_stream('fixture/claude-haiku', 'claude-opus wrote this'))
        self.assertEqual(result['code'], 'unexpected_model')
        self.assertFalse((directory / 'candidate.md').exists())

    def test_a_model_list_or_a_zero_exit_without_a_completed_answer_is_not_work(self):
        result, directory = self.execute(json.dumps({'models': [{'id': 'gemini-4-hydrogen-high'}]}))
        self.assertEqual((result['code'], result['availability']), ('completion_unverified', 'unverified'))
        self.assertFalse((directory / 'candidate.md').exists())

    def test_provider_blocks_are_reported_and_nothing_else_is_tried(self):
        for error, expected in [('USAGE_PRICING_REQUIRED', 'pricing_required'),
                                ('HTTP 402 Insufficient credits', 'insufficient_credits')]:
            self.draft()
            result, directory = self.execute('', error, 1)
            self.assertEqual((result['code'], result['availability']), (expected, 'blocked'))
            self.assertFalse((directory / 'candidate.md').exists())

    def test_incomplete_repeated_or_tool_using_runs_are_refused(self):
        tool = [{'type': 'toolCall', 'name': 'write'}, {'type': 'text', 'text': 'x'}]
        for data in [omp_stream(ended=False), omp_stream(stop='length'), omp_stream(stop='error'),
                     omp_stream() + omp_stream(), omp_stream(content=tool), omp_stream(text='  ')]:
            checked, candidate = writing.evidence('omp', data, '', 0, OMP_MODEL)
            self.assertNotEqual(checked['code'], 'candidate_generated', data)
            self.assertIsNone(candidate)
        for data in [agy_stream(status='ERROR'), agy_stream(finished_reason='print_timeout'), agy_stream(turns=2),
                     agy_stream(steps=('tool_call', 'agent_response')), agy_stream() + agy_stream(),
                     agy_stream(text=' \n'), agy_stream().replace('"conversation_id": "c1", "status"',
                                                                   '"conversation_id": "c2", "status"')]:
            checked, candidate = writing.evidence('agy', data, '', 0, AGY_MODEL)
            self.assertNotEqual(checked['code'], 'candidate_generated', data)
            self.assertIsNone(candidate)
        for code in (None, False):
            self.assertEqual(writing.evidence('agy', agy_stream(), '', code, AGY_MODEL), ({'code': 'execution_unknown'}, None))

    def test_a_run_is_never_started_twice(self):
        _, directory = self.execute()
        with self.assertRaisesRegex(ValueError, 'run_already_started'):
            writing.run(self.ach, 'fixture-contest', self.request, lambda *a, **k: self.fail('replayed'))
        self.assertTrue(writing.check(directory)['verified'])

    def test_submitted_or_not_running_contests_are_refused_before_any_output(self):
        for state, submissions in [('submitted', [{}]), ('running', [{}]), ('awaiting_approval', [])]:
            self.entry.update(state=state, submissions=submissions)
            with self.assertRaisesRegex(ValueError, 'writing_requires_unsubmitted_running'):
                writing.run(self.ach, 'fixture-contest', self.request, lambda *a, **k: self.fail('executed'))
        self.assertFalse((self.folder / '.private').exists())

    def test_inputs_outside_the_folder_or_behind_a_symlink_are_refused(self):
        (self.folder / 'link.md').symlink_to(self.source)
        (self.folder / 'binary.md').write_bytes(b'\xff\xfe\x00')
        for source in ['../outside.md', 'link.md', 'binary.md', '', None]:
            self.request['source'] = source
            with self.assertRaises(ValueError):
                writing.run(self.ach, 'fixture-contest', self.request, lambda *a, **k: self.fail('executed'))
        self.assertFalse((self.folder / '.private').exists())

    def test_a_timeout_or_an_interruption_leaves_an_unknown_run_that_is_not_repeated(self):
        def timeout(*args, **kwargs):
            raise subprocess.TimeoutExpired('agy', 1)
        result = writing.run(self.ach, 'fixture-contest', self.request, timeout)
        self.assertEqual(result['code'], 'execution_unknown')
        with self.assertRaisesRegex(ValueError, 'run_already_started'):
            writing.run(self.ach, 'fixture-contest', self.request, timeout)

        def interrupted(*args, **kwargs):
            raise KeyboardInterrupt()
        self.request['run_id'] = str(uuid.uuid4())
        with self.assertRaises(KeyboardInterrupt):
            writing.run(self.ach, 'fixture-contest', self.request, interrupted)
        directory = self.folder / '.private/writing' / self.request['run_id']
        self.assertEqual(writing.check(directory)['code'], 'execution_unknown')
        with self.assertRaisesRegex(ValueError, 'run_already_started'):
            writing.run(self.ach, 'fixture-contest', self.request, interrupted)

    def test_a_source_changed_during_the_run_yields_no_candidate(self):
        def runner(argv, **kwargs):
            self.source.write_text('edited meanwhile\n')
            return subprocess.CompletedProcess(argv, 0, stdout=agy_stream(), stderr='')
        result = writing.run(self.ach, 'fixture-contest', self.request, runner)
        self.assertEqual(result['code'], 'source_changed')
        self.assertNotIn('candidate.md', result['files'])

    def test_changed_policy_events_or_receipt_do_not_verify(self):
        self.draft()
        for name, text in [('runner-policy.json', '{"retry":{"modelFallback":true}}'),
                           ('runner-guard.ts', 'export default function () {}\n'),
                           ('stdout.jsonl', omp_stream('fixture/claude-haiku'))]:
            self.draft()
            _, directory = self.execute()
            (directory / name).write_text(text)
            self.assertFalse(writing.check(directory)['verified'], name)
            # Even with the receipt's hash brought in line, the content itself is checked.
            receipt = json.loads((directory / 'receipt.json').read_text())
            receipt['files'][name] = writing.sha(text.encode())
            (directory / 'receipt.json').write_text(json.dumps(receipt))
            self.assertFalse(writing.check(directory)['verified'], name)
        self.assertFalse(writing.check(self.folder / 'missing')['verified'])


if __name__ == '__main__':
    unittest.main()

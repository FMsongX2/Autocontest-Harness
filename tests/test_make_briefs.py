"""Controller briefs (`python3 -m controllers briefs`, module `controllers/briefs.py`).

The generated text is data (`data/queue-master/brief-overrides.json`); these tests build their own
template and only use temporary files. They never call `ach` and never skip.
"""
import json
import os
import stat
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from controllers import briefs as mb  # noqa: E402

COMMON = '## 필수 운영 규칙(시험)\n\n- 상금이 있는 공모전만 진행한다.\n'
TAIL = '## 절차\n\n시험 절차.\n'
TEMPLATE = mb.Template(version='2026-10-25.1', regions={'common': COMMON, 'procedure': TAIL},
                       special={'c1': '## 이 공모전의 특이사항\n\n- 데이터에서 온 특이사항.\n'})


class RefreshTest(unittest.TestCase):
    def setUp(self):
        self.head = '# 실행 인계: 예시 공모전\n\n'
        self.summary = '## 공모전 요약(게이트 기록, 데이터)\n\n- 사용자가 덧붙인 메모: 그대로 남아야 한다.\n\n'
        self.special = '## 이 공모전의 특이사항\n\n- 공모전마다 다른 내용.\n\n'

    def managed(self, common_body, tail_body, version=TEMPLATE.version):
        return (self.head + mb.region('common', common_body, version) + '\n' + self.summary + self.special
                + mb.region('procedure', tail_body, version))

    def legacy(self):
        return (self.head + '## 필수 운영 규칙(옛 버전)\n\n- 옛 공통 규칙.\n\n' + self.summary + self.special
                + '## 절차\n\n옛 절차.\n')

    def test_current_brief_is_left_alone(self):
        statuses, new, replaced = mb.refresh_text(self.managed(COMMON, TAIL), TEMPLATE)
        self.assertEqual(statuses, {'common': 'current', 'procedure': 'current'})
        self.assertIsNone(new)
        self.assertEqual(replaced, {})

    def test_version_change_alone_rewrites_markers_only(self):
        text = self.managed(COMMON, TAIL, version='old')
        statuses, new, _ = mb.refresh_text(text, TEMPLATE)
        self.assertEqual(statuses, {'common': 'updated', 'procedure': 'updated'})
        self.assertEqual(new, self.managed(COMMON, TAIL))

    def test_outdated_regions_are_rewritten_and_everything_else_kept(self):
        text = self.managed('## 필수 운영 규칙\n\n- 옛 규칙.\n', '## 절차\n\n옛 절차.\n')
        statuses, new, _ = mb.refresh_text(text, TEMPLATE)
        self.assertEqual(statuses, {'common': 'updated', 'procedure': 'updated'})
        self.assertEqual(new, self.managed(COMMON, TAIL))
        self.assertIn(self.summary + self.special, new)
        again, newer, _ = mb.refresh_text(new, TEMPLATE)
        self.assertEqual(again, {'common': 'current', 'procedure': 'current'})
        self.assertIsNone(newer)

    def test_hand_edited_region_is_reported_not_overwritten(self):
        text = self.managed('## 필수 운영 규칙\n\n- 옛 규칙.\n', TAIL)
        edited = text.replace('- 옛 규칙.', '- 사용자가 고친 규칙.')
        statuses, new, _ = mb.refresh_text(edited, TEMPLATE)
        self.assertEqual(statuses['common'], 'modified')
        self.assertEqual(statuses['procedure'], 'current')
        self.assertIsNone(new)

    def test_legacy_brief_is_only_reported_without_adoption(self):
        statuses, new, replaced = mb.refresh_text(self.legacy(), TEMPLATE)
        self.assertEqual(statuses, {'common': 'legacy', 'procedure': 'legacy'})
        self.assertIsNone(new)
        self.assertEqual(replaced, {})

    def test_adopting_legacy_replaces_only_the_generated_sections(self):
        statuses, new, replaced = mb.refresh_text(self.legacy(), TEMPLATE, adopt_legacy=True)
        self.assertEqual(statuses, {'common': 'adopted', 'procedure': 'adopted'})
        self.assertEqual(new, self.managed(COMMON, TAIL))
        self.assertEqual(replaced['common'], '## 필수 운영 규칙(옛 버전)\n\n- 옛 공통 규칙.\n\n')
        self.assertEqual(replaced['procedure'], '## 절차\n\n옛 절차.\n')

    def test_ambiguous_or_broken_briefs_need_review(self):
        doubled = self.legacy().replace('## 절차', '## 필수 운영 규칙(복사본)\n\n## 절차')
        statuses, new, _ = mb.refresh_text(doubled, TEMPLATE, adopt_legacy=True)
        self.assertEqual(statuses['common'], 'unrecognized')
        self.assertEqual(statuses['procedure'], 'adopted')
        self.assertNotIn('<!-- controller-brief:common', new)
        broken = self.managed(COMMON, TAIL).replace('<!-- controller-brief:common end -->\n', '')
        statuses, new, _ = mb.refresh_text(broken, TEMPLATE)
        self.assertEqual(statuses['common'], 'malformed')

    def test_refresh_file_dry_run_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'controller-brief.md'
            path.write_text(self.legacy(), encoding='utf-8')
            before = path.read_bytes()
            report = mb.refresh_file(path, TEMPLATE, dry_run=True, adopt_legacy=True)
            self.assertTrue(report['changed'])
            self.assertFalse(report['review'])
            self.assertEqual(path.read_bytes(), before)
            self.assertEqual(sorted(os.listdir(tmp)), ['controller-brief.md'])

    def test_refresh_file_keeps_mode_and_backs_up_adopted_text(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'controller-brief.md'
            path.write_text(self.legacy(), encoding='utf-8')
            os.chmod(path, 0o640)
            now = datetime(2026, 10, 25, 12, 0, tzinfo=timezone.utc)
            report = mb.refresh_file(path, TEMPLATE, adopt_legacy=True, now=now)
            self.assertEqual(path.read_text(encoding='utf-8'), self.managed(COMMON, TAIL))
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o640)
            backup = Path(report['backup'])
            self.assertEqual(backup.name, 'controller-brief.legacy-20261025T120000Z.md')
            self.assertEqual(stat.S_IMODE(backup.stat().st_mode), 0o600)
            self.assertIn('- 옛 공통 규칙.', backup.read_text(encoding='utf-8'))
            self.assertEqual(sorted(os.listdir(tmp)),
                             ['controller-brief.legacy-20261025T120000Z.md', 'controller-brief.md'])
            second = mb.refresh_file(path, TEMPLATE, adopt_legacy=True, now=now)
            self.assertFalse(second['changed'])
            self.assertEqual(second['regions'], {'common': 'current', 'procedure': 'current'})

    def test_modified_file_is_not_touched(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'controller-brief.md'
            text = self.managed(COMMON, TAIL).replace('상금이 있는', '사용자가 고친 상금')
            path.write_text(text, encoding='utf-8')
            report = mb.refresh_file(path, TEMPLATE)
            self.assertTrue(report['review'])
            self.assertFalse(report['changed'])
            self.assertEqual(path.read_text(encoding='utf-8'), text)

    def test_missing_brief_needs_review(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = mb.refresh_file(Path(tmp) / 'nope.md', TEMPLATE)
            self.assertTrue(report['review'])
            self.assertEqual(report['regions'], {'common': 'missing', 'procedure': 'missing'})


class TemplateDataTest(unittest.TestCase):
    """The operator text comes from private data; the public module holds no contest facts."""

    def home(self, overrides=None):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        home = Path(tmp.name)
        if overrides is not None:
            path = home / mb.OVERRIDES
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps(overrides, ensure_ascii=False), encoding='utf-8')
        return home

    def test_overrides_file_supplies_regions_version_and_special_sections(self):
        template = mb.load_template(self.home({'version': '2026-10-25.1', 'common': COMMON, 'procedure': TAIL,
                                               'special': TEMPLATE.special, 'launch_handoff': 'x'}))
        self.assertEqual(template, TEMPLATE)

    def test_defaults_apply_without_private_data(self):
        template = mb.load_template(self.home())
        self.assertEqual(template.version, mb.DEFAULT_VERSION)
        self.assertEqual(template.special, {})
        self.assertTrue(template.regions['common'].startswith(mb.HEADINGS['common']))

    def test_invalid_overrides_are_refused(self):
        for bad in ([], {'common': '# 다른 제목\n'}, {'special': {'c1': 3}}, {'version': 'two words'}):
            with self.assertRaises(ValueError):
                mb.load_template(self.home(bad))

    def test_render_takes_special_text_from_the_template(self):
        entry = {'title': '예시', 'type': 'paper', 'level': 3, 'deadline': None, 'url': 'https://example.org',
                 'gate': {'prize': 'x', 'deliverables': [], 'criteria': []}}
        body = mb.render(entry, 'c1', TEMPLATE)
        self.assertIn('- 데이터에서 온 특이사항.', body)
        self.assertNotIn('특이사항', mb.render(entry, 'c2', TEMPLATE))
        statuses, new, _ = mb.refresh_text(body, TEMPLATE)
        self.assertEqual(statuses, {'common': 'current', 'procedure': 'current'})
        self.assertIsNone(new)

    def test_public_module_holds_no_operator_specific_text(self):
        source = (ROOT / 'controllers' / 'briefs.py').read_text(encoding='utf-8')
        for private in ('/Users/', 'accounts.md', 'thinkcontest-', 'scout-', 'Vultr'):
            self.assertNotIn(private, source)


if __name__ == '__main__':
    unittest.main()

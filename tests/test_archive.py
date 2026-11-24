"""`controllers.archive` against real local Git repositories. No hosting service, account or network."""
import json
import os
import subprocess
import tempfile
import unittest
import uuid
from pathlib import Path
from unittest.mock import patch

from controllers import archive


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve()
        self.env = patch.dict(os.environ, {'GIT_AUTHOR_NAME': 'Fixture', 'GIT_COMMITTER_NAME': 'Fixture',
                                          'GIT_AUTHOR_EMAIL': 'fixture@example.invalid',
                                          'GIT_COMMITTER_EMAIL': 'fixture@example.invalid'})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.remote = self.home / 'remote.git'
        self.seed = self.home / 'seed'
        self.seed.mkdir()
        self.g(self.home, 'init', '--bare', '--initial-branch=main', str(self.remote))
        self.g(self.seed, 'init', '--initial-branch=main')
        self.g(self.seed, 'remote', 'add', 'origin', str(self.remote))
        old = self.seed / 'contests/previous'
        old.mkdir(parents=True)
        (old / 'README.md').write_text('Previously submitted; preserve exactly.\n')
        self.old_row = {'id': 'previous', 'path': 'contests/previous', 'state': 'submitted',
                        'custom': {'preserve': [1, 2]}, 'submitted_at': '2026-01-01'}
        self.legacy = {'path': 'contests/opaque', 'custom': 'no registry id'}
        (self.seed / 'index.json').write_text(json.dumps({'contests': [self.old_row, self.legacy], 'custom': 'root'}))
        (self.seed / 'README.md').write_text('Existing archive overview.\n')
        self.g(self.seed, 'add', '.')
        self.g(self.seed, 'commit', '-m', 'Fixture')
        self.g(self.seed, 'push', 'origin', 'main')
        self.before = self.g(self.remote, 'rev-parse', 'main')
        self.old_tree = self.g(self.remote, 'rev-parse', 'main:contests/previous')
        self.source = self.home / 'entry'
        self.source.mkdir()
        (self.source / '원고.md').write_text('Public, reviewed source with 123 facts.\n')
        self.entry = {'id': 'selected', 'state': 'running', 'folder': str(self.source), 'submissions': [],
                      'title': 'Example contest', 'organizer': 'Example organizer', 'url': 'https://example.invalid/contest',
                      'type': 'report', 'level': 2, 'deadline': None, 'history': []}
        self.calls = []
        owner = self

        class Ach:
            root = owner.home

            def json(self, *args):
                owner.calls.append(args)
                assert args[0] == 'show'
                return dict(owner.entry)
        self.ach = Ach()

    def g(self, folder, *args):
        return archive.git(folder, *args)

    def request(self):
        return {'run_id': str(uuid.uuid4()), 'phase': 'assignment', 'remote': str(self.remote),
                'private_remote_confirmed': True, 'selection': [{'id': 'selected', 'files': [
                    {'path': '원고.md', 'sha256': archive.digest((self.source / '원고.md').read_bytes()),
                     'privacy_reviewed': True}]}]}

    def prepared(self, request=None):
        request = request or self.request()
        result = archive.prepare(self.ach, request)
        return result, self.home / 'data/archive-sync' / result['run_id']

    def remote_index(self):
        return json.loads(self.g(self.remote, 'show', 'main:index.json'))

    def test_selected_sync_preserves_prior_tree_and_rows_and_verifies_remote(self):
        prepared, directory = self.prepared()
        self.assertEqual(prepared['state'], 'prepared')
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), self.before)
        published = archive.publish(self.ach, directory)
        self.assertEqual(published['state'], 'verified')
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), published['commit'])
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main:contests/previous'), self.old_tree)
        self.assertEqual(self.remote_index()['contests'][:2], [self.old_row, self.legacy])
        self.assertEqual(self.remote_index()['custom'], 'root')
        self.assertIn('Existing archive overview.', self.g(self.remote, 'show', 'main:README.md'))
        self.assertEqual({call[0] for call in self.calls}, {'show'})
        with self.assertRaisesRegex(archive.ArchiveError, 'push_not_replayable'):
            archive.publish(self.ach, directory)
        same, _ = self.prepared()
        self.assertEqual(same['state'], 'unchanged', 'timestamps alone must not produce another commit')

    def test_concurrent_preparations_use_separate_dirs_and_second_push_is_not_attempted(self):
        first, one = self.prepared()
        second, two = self.prepared()
        self.assertNotEqual(one, two)
        archive.publish(self.ach, one)
        seen = []

        def runner(argv, **kwargs):
            seen.append(argv)
            return subprocess.run(argv, **kwargs)
        result = archive.publish(self.ach, two, runner)
        self.assertEqual(result['state'], 'conflict')
        self.assertFalse(any('push' in argv for argv in seen))
        self.assertTrue(one.is_dir() and two.is_dir())
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), first['commit'])

    def test_lost_push_response_can_be_verified_without_replay(self):
        _, directory = self.prepared()
        pushes = []

        def runner(argv, **kwargs):
            if 'push' in argv:
                pushes.append(argv)
                subprocess.run(argv, **kwargs)
                raise subprocess.TimeoutExpired(argv, 60)
            if 'fetch' in argv:
                raise subprocess.TimeoutExpired(argv, 60)
            return subprocess.run(argv, **kwargs)
        result = archive.publish(self.ach, directory, runner)
        self.assertEqual((result['state'], result['reason']), ('unknown', 'remote_unreachable'))
        self.assertEqual(len(pushes), 1)
        with self.assertRaisesRegex(archive.ArchiveError, 'push_not_replayable'):
            archive.publish(self.ach, directory, runner)
        (self.source / '원고.md').write_text('New reviewed version after the uncertain attempt\n')
        _, next_directory = self.prepared()
        with self.assertRaisesRegex(archive.ArchiveError, 'prior_push_unresolved'):
            archive.publish(self.ach, next_directory, runner)
        self.assertEqual(archive.verify(directory)['state'], 'verified')
        self.assertEqual(len(pushes), 1)

    def test_lease_refuses_a_race_after_the_pre_push_observation(self):
        _, directory = self.prepared()

        def runner(argv, **kwargs):
            if 'push' in argv:
                (self.seed / 'other.txt').write_text('Concurrent owner change\n')
                self.g(self.seed, 'add', 'other.txt')
                self.g(self.seed, 'commit', '-m', 'Concurrent fixture')
                self.g(self.seed, 'push', 'origin', 'main')
            return subprocess.run(argv, **kwargs)
        result = archive.publish(self.ach, directory, runner)
        # The branch moved on without the commit: the leased push can never land, so the run is settled.
        self.assertEqual((result['state'], result['reason']), ('not_published', 'remote_advanced_without_commit'))
        self.assertEqual(self.g(self.remote, 'show', 'main:other.txt'), 'Concurrent owner change')
        self.assertEqual(self.remote_index()['contests'], [self.old_row, self.legacy])
        self.assertEqual(archive.verify(directory)['state'], 'not_published')
        # It does not hold back a new run for the same contest on the new base.
        _, again = self.prepared()
        self.assertEqual(archive.publish(self.ach, again)['state'], 'verified')

    def test_a_push_that_never_arrived_stays_unknown_until_it_is_abandoned(self):
        _, directory = self.prepared()
        pushes = []

        def runner(argv, **kwargs):
            if 'push' in argv:
                pushes.append(argv)
                raise subprocess.TimeoutExpired(argv, 60)
            return subprocess.run(argv, **kwargs)
        result = archive.publish(self.ach, directory, runner)
        # The remote still is the base: the push could yet arrive, so nothing is concluded.
        self.assertEqual((result['state'], result['reason']), ('unknown', 'commit_not_on_remote'))
        self.assertEqual(result['observed_remote'], self.before)
        _, blocked = self.prepared()
        with self.assertRaisesRegex(archive.ArchiveError, 'prior_push_unresolved'):
            archive.publish(self.ach, blocked, runner)
        self.assertEqual(archive.abandon(directory)['state'], 'abandoned')
        self.assertEqual(archive.verify(directory)['state'], 'abandoned')
        self.assertEqual(archive.publish(self.ach, blocked)['state'], 'verified')
        self.assertEqual(len(pushes), 1)
        # Abandoning is for an unknown outcome only: a published run stays published.
        self.assertEqual(archive.abandon(blocked)['state'], 'verified')
        with self.assertRaisesRegex(archive.ArchiveError, 'no_push_to_verify'):
            archive.abandon(self.prepared(dict(self.request(), remote=str(self.remote)))[1])

    def test_a_published_update_changed_again_by_a_later_commit_is_superseded_not_unknown(self):
        _, directory = self.prepared()
        changed = False

        def runner(argv, **kwargs):
            nonlocal changed
            if 'fetch' in argv and not changed:
                changed = True
                other = self.home / 'later-writer'
                self.g(self.home, 'clone', str(self.remote), str(other))
                (other / 'contests/selected/원고.md').write_text('Rewritten by a later commit\n')
                self.g(other, 'add', '.')
                self.g(other, 'commit', '-m', 'Later change')
                self.g(other, 'push', 'origin', 'main')
            return subprocess.run(argv, **kwargs)
        receipt = archive.publish(self.ach, directory, runner)
        self.assertEqual((receipt['state'], receipt['reason']), ('superseded', 'selected_content_changed_later'))

    def test_remote_descendant_with_unrelated_changes_still_proves_the_selected_update(self):
        _, directory = self.prepared()
        advanced = False

        def runner(argv, **kwargs):
            nonlocal advanced
            if 'fetch' in argv and not advanced:
                advanced = True
                other = self.home / 'other-writer'
                self.g(self.home, 'clone', str(self.remote), str(other))
                (other / 'unrelated.txt').write_text('Another writer\n')
                self.g(other, 'add', 'unrelated.txt')
                self.g(other, 'commit', '-m', 'Unrelated change')
                self.g(other, 'push', 'origin', 'main')
            return subprocess.run(argv, **kwargs)
        receipt = archive.publish(self.ach, directory, runner)
        self.assertEqual(receipt['state'], 'verified')
        self.assertNotEqual(receipt['commit'], receipt['remote_commit'])
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main:contests/previous'), self.old_tree)

    def test_submitted_source_or_submitted_destination_is_protected(self):
        self.entry['submissions'] = [{}]
        with self.assertRaisesRegex(archive.ArchiveError, 'protected'):
            self.prepared()
        self.entry['submissions'] = []
        self.entry['id'] = 'previous'
        request = self.request()
        request['selection'][0]['id'] = 'previous'
        with self.assertRaisesRegex(archive.ArchiveError, 'submitted_archive_protected'):
            self.prepared(request)
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), self.before)

    def test_privacy_exclusions_hashes_and_symlinks_refuse_the_whole_selection(self):
        cases = [('notes.md', 'mail person@example.invalid'), ('.private/notes.md', 'private'),
                 ('signature.txt', 'signature'), ('picture.png', 'image')]
        for name, content in cases:
            path = self.source / name
            path.parent.mkdir(exist_ok=True)
            path.write_text(content)
            request = self.request()
            request['selection'][0]['files'].append({'path': name, 'sha256': archive.digest(path.read_bytes()), 'privacy_reviewed': True})
            with self.assertRaises(archive.ArchiveError):
                self.prepared(request)
        request = self.request()
        request['selection'][0]['files'][0]['sha256'] = '0' * 64
        with self.assertRaisesRegex(archive.ArchiveError, 'source_changed'):
            self.prepared(request)
        (self.source / 'link.md').symlink_to(self.source / '원고.md')
        request['selection'][0]['files'] = [{'path': 'link.md', 'privacy_reviewed': True}]
        with self.assertRaisesRegex(archive.ArchiveError, 'file_not_regular'):
            self.prepared(request)
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), self.before)

    def test_source_or_workflow_change_after_prepare_refuses_publish(self):
        _, directory = self.prepared()
        self.entry['state'] = 'submitted'
        with self.assertRaisesRegex(archive.ArchiveError, 'protected'):
            archive.publish(self.ach, directory)
        self.entry['state'] = 'running'
        (self.source / '원고.md').write_text('Changed since privacy review\n')
        with self.assertRaisesRegex(archive.ArchiveError, 'source_changed'):
            archive.publish(self.ach, directory)
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), self.before)

    def test_followup_does_not_mutate_workflow_and_ack_does_not_erase_newer_event(self):
        root = self.home / 'data/archive-sync'
        root.mkdir(parents=True)
        (root / 'config.json').write_text('{"enabled":true}')
        first = archive.enqueue(self.ach, 'selected', 'assignment')
        self.assertEqual(first['state'], 'pending')
        pending = root / 'pending/selected.json'
        before = pending.stat().st_mtime_ns
        archive.enqueue(self.ach, 'selected', 'assignment')
        self.assertEqual(pending.stat().st_mtime_ns, before)
        _, directory = self.prepared()
        # A newer notification arrives while publishing the prepared snapshot.
        archive.enqueue(self.ach, 'selected', 'checkpoint')
        published = archive.publish(self.ach, directory)
        self.assertEqual(published['state'], 'verified')
        self.assertEqual(json.loads(pending.read_text())['state'], 'pending')
        same, _ = self.prepared()
        self.assertEqual(same['state'], 'unchanged')
        self.assertEqual(json.loads(pending.read_text())['state'], 'verified')
        self.assertEqual(self.entry['state'], 'running')
        self.assertEqual({call[0] for call in self.calls}, {'show'})

    def test_enqueue_failure_is_explicit_and_does_not_fail_the_completed_assignment(self):
        root = self.home / 'data/archive-sync'
        root.mkdir(parents=True)
        (root / 'config.json').write_text('broken json')
        self.assertEqual(archive.enqueue(self.ach, 'selected', 'assignment')['state'], 'error')
        self.assertEqual(self.calls, [])
        self.assertEqual(self.entry['state'], 'running')

    def test_a_remote_that_serves_partial_clones_is_published_and_verified_without_its_file_contents(self):
        # As a hosting service does: the clone takes one commit and no blobs, and fetches the few it reads.
        for key in ('uploadpack.allowFilter', 'uploadpack.allowAnySHA1InWant'):
            self.g(self.remote, 'config', key, 'true')
        request = self.request()
        request['remote'] = 'file://' + str(self.remote)
        prepared, directory = self.prepared(request)
        self.assertEqual(prepared['state'], 'prepared')
        repo = directory / 'repo'
        self.assertEqual(self.g(repo, 'rev-parse', '--is-shallow-repository'), 'true')
        missing = self.g(repo, 'rev-list', '--objects', '--missing=print', 'HEAD').splitlines()
        self.assertTrue(any(line.startswith('?') for line in missing), 'unselected file contents were not fetched')
        published = archive.publish(self.ach, directory)
        self.assertEqual(published['state'], 'verified')
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), published['commit'])
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main:contests/previous'), self.old_tree)
        self.assertEqual(self.g(self.remote, 'show', 'main:contests/selected/원고.md'),
                         'Public, reviewed source with 123 facts.')
        self.g(self.remote, 'fsck', '--strict')
        request = self.request()
        request['remote'] = 'file://' + str(self.remote)
        self.assertEqual(self.prepared(request)[0]['state'], 'unchanged')

    def test_a_new_row_takes_the_archive_s_own_columns_and_an_existing_row_keeps_its_own(self):
        request = self.request()
        request['row_defaults'] = {'submitted_at': '', 'final_score': None}
        _, directory = self.prepared(request)
        archive.publish(self.ach, directory)
        row = self.remote_index()['contests'][2]
        self.assertEqual((row['id'], row['path'], row['files'], row['title']),
                         ('selected', 'contests/selected', 1, 'Example contest'))
        self.assertEqual((row['submitted_at'], row['final_score']), ('', None))
        self.assertNotIn('history', row)
        # An update keeps what the row already had and does not apply defaults again.
        (self.source / '원고.md').write_text('Second reviewed version.\n')
        request = self.request()
        request['row_defaults'] = {'submitted_at': 'never applied'}
        _, directory = self.prepared(request)
        archive.publish(self.ach, directory)
        self.assertEqual(self.remote_index()['contests'][2]['submitted_at'], '')
        for bad in ({'id': 'other'}, {'path': 'contests/x'}, {'Note': 'x'}, {'extra': ['list']},
                    {'contact': 'person@example.invalid'}):
            request = self.request()
            request['row_defaults'] = bad
            with self.assertRaises(archive.ArchiveError):
                self.prepared(request)

    def test_remotes_with_credentials_other_transports_or_without_the_privacy_confirmation_are_refused(self):
        for change in ({'remote': 'https://user:secret@example.invalid/archive.git'},
                       {'remote': 'ext::sh -c id'}, {'remote': '--upload-pack=id'}, {'remote': 'relative/path'},
                       {'branch': '../main'}, {'branch': '-x'}, {'private_remote_confirmed': False},
                       {'run_id': 'not-a-uuid'}, {'phase': 'submission'}):
            request = dict(self.request(), **change)
            with self.assertRaises(archive.ArchiveError):
                self.prepared(request)
        self.assertFalse((self.home / 'data/archive-sync').exists())
        self.assertEqual(self.g(self.remote, 'rev-parse', 'main'), self.before)


if __name__ == '__main__':
    unittest.main()

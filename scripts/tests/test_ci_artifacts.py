import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('ci_artifacts', Path(__file__).parents[1] / 'ci-artifacts.py')
artifacts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(artifacts)


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.archive = self.root / 'package.zip'
        self.source = self.root / 'app.dmg'
        self.source.write_bytes(b'package bytes')
        self.commit = 'a' * 40

    def test_roundtrip(self):
        artifacts.pack(self.archive, [self.source], self.commit)
        destination = self.root / 'download'
        artifacts.unpack(self.archive, destination, self.commit)
        self.assertEqual((destination / 'app.dmg').read_bytes(), self.source.read_bytes())

    def test_wrong_commit_rejected_before_extract(self):
        artifacts.pack(self.archive, [self.source], self.commit)
        with self.assertRaises(ValueError):
            artifacts.unpack(self.archive, self.root / 'download', 'b' * 40)
        self.assertFalse((self.root / 'download').exists())

    def test_duplicate_input_rejected(self):
        with self.assertRaises(ValueError):
            artifacts.pack(self.archive, [self.source, self.source], self.commit)

    def test_tampered_archive_and_traversal_rejected(self):
        for name, digest in [('app.dmg', 'wrong hash'), ('../outside', 'wrong hash')]:
            with self.subTest(name=name):
                with zipfile.ZipFile(self.archive, 'w') as archive:
                    archive.writestr(name, b'changed')
                    archive.writestr('.ci-artifact.json', json.dumps({'commit': self.commit, 'files': {name: digest}}))
                with self.assertRaises(ValueError):
                    artifacts.unpack(self.archive, self.root / 'download', self.commit)
                self.assertFalse((self.root / 'download').exists())

    def test_public_repository_rejected(self):
        with patch.dict('os.environ', {'GITHUB_REPOSITORY': 'owner/repo', 'GITHUB_RUN_ID': '123', 'GITHUB_SHA': self.commit}), patch.object(artifacts, 'gh', return_value='false'):
            with self.assertRaises(ValueError):
                artifacts.context()

    def test_published_or_wrong_target_draft_rejected(self):
        for draft, target in [(False, self.commit), (True, 'b' * 40)]:
            with patch.object(artifacts, 'gh', return_value=json.dumps({'isDraft': draft, 'targetCommitish': target})):
                with self.assertRaises(ValueError):
                    artifacts.verify_draft('owner/repo', 'ci-artifacts-123', self.commit)


if __name__ == '__main__':
    unittest.main()

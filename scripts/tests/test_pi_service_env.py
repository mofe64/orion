from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pi_service_config import merge_env, read_env


class MergeEnvTests(unittest.TestCase):
    def test_update_moves_release_path_instead_of_dropping_it(self):
        old = 'ORION_LLAMA_SERVER="/releases/old/native/llama-server"\nORION_ASR_THREADS="2"\n'
        merged = read_env(merge_env(old, {'ORION_LLAMA_SERVER': '/releases/new/native/llama-server',
                                          'ORION_ASR_THREADS': '3'}, {'ORION_LLAMA_SERVER'}))
        self.assertEqual(merged['ORION_LLAMA_SERVER'], '/releases/new/native/llama-server')
        self.assertEqual(merged['ORION_ASR_THREADS'], '2')

    def test_removed_key_without_default_stays_removed(self):
        merged = read_env(merge_env('OLD_KEY="x"\n', {}, {'OLD_KEY'}))
        self.assertNotIn('OLD_KEY', merged)


if __name__ == '__main__': unittest.main()

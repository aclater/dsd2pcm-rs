# SPDX-License-Identifier: MIT OR Apache-2.0
"""Tests of analyse.py's pure functions: python -m unittest discover -s tools/quality"""

import unittest

import analyse


class RelateToSources(unittest.TestCase):
    def test_level_steps_in_sources(self):
        # Level steps (and frequency and crosstalk steps) have no in_hz, in the source's
        # analysis and in a decoder's; --sources level_steps used to raise KeyError: 'in_hz'.
        level = {"steps": [{"expected_dbfs": -6.0, "level_dbfs": -6.02, "tone_dbfs": -6.02}]}
        results = {
            "source/level_steps": {"steps": [dict(level["steps"][0])]},
            "dsd2pcm/176k4-40/level_steps": {"steps": [dict(level["steps"][0])]},
        }
        analyse.relate_to_sources(results)
        self.assertNotIn("rejection_re_source_db", results["dsd2pcm/176k4-40/level_steps"]["steps"][0])

    def test_alias_steps_related(self):
        results = {
            "source/alias_steps": {"steps": [{"in_hz": 50000.0, "tone_dbfs": -20.0},
                                             {"in_hz": 60000.0, "tone_dbfs": -21.0}]},
            "dsd2pcm/88k2-40/alias_steps": {"steps": [{"in_hz": 60000.0, "out_dbfs": -150.0},
                                                      {"in_hz": 70000.0, "out_dbfs": -149.0}]},
        }
        analyse.relate_to_sources(results)
        steps = results["dsd2pcm/88k2-40/alias_steps"]["steps"]
        self.assertEqual(steps[0]["source_tone_dbfs"], -21.0)
        self.assertAlmostEqual(steps[0]["rejection_re_source_db"], 129.0)
        self.assertNotIn("rejection_re_source_db", steps[1])  # no source tone at 70 kHz


if __name__ == "__main__":
    unittest.main()

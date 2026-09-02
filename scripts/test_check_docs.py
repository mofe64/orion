import unittest

from scripts import check_docs


class ReaderFirstLanguageTests(unittest.TestCase):
    def test_rejects_meta_openings(self) -> None:
        for opening in (
            "This page explains Orion motion.",
            "This tutorial covers setup.",
            "This document describes the runtime.",
            "This package owns motion assets.",
            "Use this guide to configure Orion.",
            "The purpose of this page is to document lighting.",
            "In this tutorial, you will run Orion.",
        ):
            with self.subTest(opening=opening):
                self.assertRegex(opening, check_docs.META_OPENING)

    def test_rejects_prohibited_reader_language(self) -> None:
        for line in (
            "Use this as the source of truth.",
            "The adapter currently requires macOS.",
            "Windows is not yet commissioned.",
            "The next release will add pairing.",
            "The new implementation supports speech.",
            "| Document contract | Value |",
        ):
            with self.subTest(line=line):
                self.assertTrue(check_docs.has_prohibited_reader_language(line))

    def test_allows_precise_sequence_words(self) -> None:
        for line in (
            "Sleep until the next deadline.",
            "The next idle starts from the same anchor.",
            "Unconfigured servos require a unique bus ID.",
        ):
            with self.subTest(line=line):
                self.assertFalse(check_docs.has_prohibited_reader_language(line))


if __name__ == "__main__":
    unittest.main()

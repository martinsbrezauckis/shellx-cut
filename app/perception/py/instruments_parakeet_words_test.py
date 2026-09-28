"""Focused contract tests for Parakeet BPE word aggregation."""
import unittest

from instruments import (
    _aggregate_parakeet_words,
    _is_nonspoken_sentence_or_quote_token,
)


def aggregate(tokens, timestamps, audio_end_ms=8_000):
    return _aggregate_parakeet_words(tokens, timestamps, 0, audio_end_ms)


class ParakeetWordAggregationTest(unittest.TestCase):
    def test_only_sentence_and_quote_tokens_are_nonspoken(self):
        for token in (".", "?!", "…", "。", "“", "”", "'"):
            with self.subTest(token=token):
                self.assertTrue(_is_nonspoken_sentence_or_quote_token(token))
        # Do not classify all Unicode punctuation: these can be spoken or carry
        # lexical meaning. A mixed BPE piece stays conservative as well.
        for token in ("%", "+", "-", "$", "&", "3.14", "friend.", "你"):
            with self.subTest(token=token):
                self.assertFalse(_is_nonspoken_sentence_or_quote_token(token))
        self.assertFalse(_is_nonspoken_sentence_or_quote_token(".", "3", "14"))

    def test_sentence_punctuation_and_quotes_preserve_text_without_extending_end(self):
        words = aggregate(
            [" “", "hello", "”", ".", " world"],
            [0.00, 0.08, 0.16, 0.24, 0.40],
            1_000,
        )
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
            [("“hello”.", 80, 400), ("world", 400, 800)],
        )
        self.assertEqual(" ".join(w["word"] for w in words), "“hello”. world")

        straight = aggregate([" \"", "hello", "\"", " world"], [0.00, 0.08, 0.16, 0.40], 1_000)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in straight],
            [("\"hello\"", 80, 400), ("world", 400, 800)],
        )

        after_word = aggregate([" said", " “", "hello"], [0.00, 1.00, 1.08], 2_000)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in after_word],
            [("said", 0, 400), ("“hello", 1_080, 1_480)],
        )

    def test_delayed_sentence_punctuation_attaches_left_and_starts_next_word(self):
        words = aggregate(
            [" sad", ".", " I'm"],
            [4.00, 4.72, 4.96],
            6_000,
        )
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
            [("sad.", 4_000, 4_400), ("I'm", 4_960, 5_360)],
        )

    def test_pending_leading_quote_keeps_following_punctuation_in_order(self):
        for tokens, expected in (
            ([" said", " “", "”", "hello"], [("said", 0, 400), ("“”hello", 1_160, 1_560)]),
            ([" said", " “", "...", "hello"], [("said", 0, 400), ("“...hello", 1_160, 1_560)]),
        ):
            with self.subTest(tokens=tokens):
                words = aggregate(tokens, [0.00, 1.00, 1.08, 1.16], 2_000)
                self.assertEqual(
                    [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
                    expected,
                )

        # Without a leading-space/gap boundary, quotes remain in the lexical
        # word they surround; text order and the lexical `hello` anchor hold.
        inline = aggregate([" said", "“", "”", "hello"], [0.00, 0.08, 0.16, 0.24], 1_000)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in inline],
            [("said“”hello", 0, 640)],
        )

    def test_orphan_punctuation_is_retained_without_extending_a_lexical_word(self):
        words = aggregate([" “", "”"], [0.00, 0.08], 1_000)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
            [("“”", 0, 400)],
        )
        trailing = aggregate([" word", " “"], [0.00, 0.80], 1_000)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in trailing],
            [("word “", 0, 400)],
        )

    def test_apostrophe_hyphen_decimal_operator_and_cjk_tokens_stay_conservative(self):
        words = aggregate(
            [" I", "'", "m", " co", "-", "op", " 3", ".", "14", " 50", "%", " 你", "好", "。", " 世界"],
            [0.00, 0.08, 0.16, 0.24, 0.32, 0.40, 0.48, 0.56, 0.64, 0.72, 0.80, 0.88, 0.96, 1.04, 1.20],
            2_000,
        )
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
            [
                ("I'm", 0, 240),
                ("co-op", 240, 480),
                ("3.14", 480, 720),
                ("50%", 720, 880),
                ("你好。", 880, 1_200),
                ("世界", 1_200, 1_600),
            ],
        )

    def test_spoken_symbols_remain_acoustic_anchors(self):
        # These are deliberately outside the sentence/quote set. With the
        # 720-ms gap to `next`, retaining their token time is observable even
        # under the unchanged 400-ms tail cap.
        words = aggregate([" 50", "%", " next", " 1", "+", " later", " co", "-", " then"],
                          [0.00, 0.08, 0.80, 1.00, 1.08, 1.80, 2.00, 2.08, 2.80], 3_000)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
            [
                ("50%", 0, 480), ("next", 800, 1_000), ("1+", 1_000, 1_480),
                ("later", 1_800, 2_000), ("co-", 2_000, 2_480), ("then", 2_800, 3_000),
            ],
        )

    def test_mixed_lexical_and_punctuation_token_remains_an_acoustic_anchor(self):
        words = aggregate([" friend.", " next"], [0.80, 1.60], 2_000)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
            [("friend.", 800, 1_200), ("next", 1_600, 2_000)],
        )

    def test_terminal_sentence_punctuation_time_never_moves_the_lexical_end(self):
        # The retained r4 replay emitted the final period at 6560 ms; a fresh
        # reviewed WSL decode emitted the same period at 7440 ms. Both must keep
        # the sentence text while anchoring the terminal word to lexical `you`.
        for period_s in (6.56, 7.44):
            with self.subTest(period_s=period_s):
                words = aggregate([" you", "."], [6.24, period_s], 7_616)
                self.assertEqual(
                    [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
                    [("you.", 6_240, 6_640)],
                )

    def test_retained_real_tokens_preserve_text_and_clear_every_affected_mms_end(self):
        # Raw tokens/timestamps: retained WSL reproduction for r4. The paired
        # independent MMS_FA diagnostic supplies the lexical end references.
        tokens = [
            " Hi", " my", " fri", "end", ".", " H", "ow", " are", " you", "?",
            " I", "'", "m", " so", " sad", ".", " I", "'", "m", " think", "ing",
            " about", " you", ".",
        ]
        timestamps = [
            0.00, 0.48, 0.80, 1.12, 1.36, 1.60, 1.68, 2.00, 2.32, 2.64,
            3.28, 3.44, 3.52, 3.68, 4.00, 4.72, 4.96, 5.04, 5.20, 5.36,
            5.60, 5.92, 6.24, 6.56,
        ]
        words = aggregate(tokens, timestamps, 7_616)
        self.assertEqual(
            [(w["word"], w["start_ms"], w["end_ms"]) for w in words],
            [
                ("Hi", 0, 400), ("my", 480, 800), ("friend.", 800, 1_520),
                ("How", 1_600, 2_000), ("are", 2_000, 2_320), ("you?", 2_320, 2_720),
                ("I'm", 3_280, 3_680), ("so", 3_680, 4_000), ("sad.", 4_000, 4_400),
                ("I'm", 4_960, 5_360), ("thinking", 5_360, 5_920),
                ("about", 5_920, 6_240), ("you.", 6_240, 6_640),
            ],
        )
        self.assertEqual(
            " ".join(w["word"] for w in words),
            "Hi my friend. How are you? I'm so sad. I'm thinking about you.",
        )
        # These are all spans changed by sentence/quote attachment. Each still
        # ends after the independently aligned MMS_FA lexical word endpoint.
        # A failing assertion names the speech-truncation risk explicitly.
        mms_ends = {2: 1_002, 5: 2_124, 8: 4_248, 12: 6_112}
        for index, aligned_end_ms in mms_ends.items():
            candidate = words[index]
            self.assertGreaterEqual(
                candidate["end_ms"],
                aligned_end_ms,
                f"{candidate['word']!r} would truncate independently aligned speech: "
                f"candidate end={candidate['end_ms']} ms, MMS_FA end={aligned_end_ms} ms",
            )


if __name__ == "__main__":
    unittest.main()

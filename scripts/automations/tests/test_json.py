import unittest

from uv_automations.json import loads


class JsonTests(unittest.TestCase):
    def test_decode_json(self) -> None:
        self.assertEqual(
            loads('{"values":[null,true,false,1,-2,1.5],"nested":{"name":"value"}}'),
            {
                "values": [None, True, False, 1, -2, 1.5],
                "nested": {"name": "value"},
            },
        )

    def test_reject_duplicate_fields(self) -> None:
        for value in [
            '{"key":1,"key":2}',
            '{"nested":{"key":1,"key":2}}',
            r'{"key":1,"\u006bey":2}',
        ]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                loads(value)

    def test_reject_non_finite_numbers(self) -> None:
        for value in ["NaN", "Infinity", "-Infinity", "1e10000", "[NaN]"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                loads(value)

import unittest

from uv_automations.json import as_boolean, loads, require_keys


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

    def test_boolean_decoder_does_not_accept_integer_aliases(self) -> None:
        self.assertIs(as_boolean(True), True)
        self.assertIs(as_boolean(False), False)
        for value in (0, 1, None, "true"):
            with self.subTest(value=value), self.assertRaises(TypeError):
                as_boolean(value)

    def test_required_fields_are_exact(self) -> None:
        require_keys({"name": "test"}, {"name"})
        for value in ({}, {"name": "test", "extra": True}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                require_keys(value, {"name"})

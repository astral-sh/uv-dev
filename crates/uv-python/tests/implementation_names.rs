use uv_python::{ImplementationName, LenientImplementationName};

#[test]
fn known_implementation_names() -> anyhow::Result<()> {
    assert_eq!(ImplementationName::default(), ImplementationName::CPython);

    for (implementation, long, short, pretty) in [
        (
            ImplementationName::CPython,
            "cpython",
            Some("cp"),
            "CPython",
        ),
        (ImplementationName::PyPy, "pypy", Some("pp"), "PyPy"),
        (
            ImplementationName::GraalPy,
            "graalpy",
            Some("gp"),
            "GraalPy",
        ),
        (ImplementationName::Pyodide, "pyodide", None, "Pyodide"),
    ] {
        assert_eq!(implementation.long_name(), long);
        assert_eq!(implementation.short_name(), short);
        assert_eq!(implementation.to_string(), long);
        assert_eq!(pretty.parse::<ImplementationName>()?, implementation);
        assert_eq!(
            serde_json::to_value(implementation)?,
            serde_json::Value::String(long.to_string())
        );

        for spelling in std::iter::once(long).chain(short) {
            for input in [spelling.to_string(), spelling.to_ascii_uppercase()] {
                assert_eq!(input.parse::<ImplementationName>()?, implementation);

                let lenient = LenientImplementationName::from(input.as_str());
                assert_eq!(lenient, LenientImplementationName::Known(implementation));
                assert_eq!(lenient, LenientImplementationName::from(implementation));
                assert_eq!(<&str>::from(&lenient), long);
                assert_eq!(lenient.pretty(), pretty);
                assert_eq!(lenient.to_string(), long);
                assert_eq!(
                    serde_json::to_value(&lenient)?,
                    serde_json::Value::String(long.to_string())
                );
            }
        }
    }

    Ok(())
}

#[test]
fn unknown_implementation_names() -> anyhow::Result<()> {
    for (input, display) in [
        ("", ""),
        (" CPython", " cpython"),
        ("cpython ", "cpython "),
        ("py", "py"),
        ("c\u{0440}ython", "c\u{0440}ython"),
        ("MY-ÇPÜ", "my-ÇpÜ"),
        ("Custom\0VM", "custom\0vm"),
    ] {
        let error = input
            .parse::<ImplementationName>()
            .expect_err("unknown implementation name");
        assert_eq!(
            error.to_string(),
            format!("Unknown Python implementation `{input}`")
        );

        let lenient = LenientImplementationName::from(input);
        assert_eq!(
            lenient,
            LenientImplementationName::Unknown(input.to_string())
        );
        assert_eq!(<&str>::from(&lenient), input);
        assert_eq!(lenient.pretty(), input);
        assert_eq!(lenient.to_string(), display);
        assert_eq!(
            serde_json::to_value(&lenient)?,
            serde_json::Value::String(input.to_string())
        );
    }

    Ok(())
}

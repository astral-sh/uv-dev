//! End-to-end coverage for independently owned sibling observations.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::uv_snapshot;

/// Withdrawing one sibling's selected version cannot withdraw another sibling's identical
/// observation. The surviving version must remain available to later live preferences.
#[test]
fn coordinated_observations_survive_retraction() -> Result<()> {
    let scenario: Scenario = toml::from_str(include_str!(
        "../../../../test/scenarios/fork/coordinated-observations-retained-version.toml"
    ))?;
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.15"
        dependencies = ["anchor; python_version == '3.12'", "fluctuating; python_version == '3.13'", "consumer-delay-000; python_version == '3.14'"]

        [tool.uv]
        fork-strategy = "fewest"
        environments = ["python_version == '3.12'", "python_version == '3.13'", "python_version == '3.14'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::RUST_LOG, "uv_resolver::resolver=debug")
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Solving with installed Python version: 3.12.[X]
    DEBUG Solving with target Python version: >=3.12, <3.15
    DEBUG Narrowed `requires-python` bound to: >=3.14, <3.15
    DEBUG Narrowed `requires-python` bound to: >=3.13, <3.14
    DEBUG Narrowed `requires-python` bound to: >=3.12, <3.13
    DEBUG Solving split (markers: python_full_version == '3.12.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.12" }, VersionSpecifier { operator: LessThan, version: "3.13" }]), range: RequiresPythonRange(LowerBound(Included("3.12")), UpperBound(Excluded("3.13"))) })
    DEBUG Adding direct dependency: project*
    DEBUG Solving split (markers: python_full_version == '3.13.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.13" }, VersionSpecifier { operator: LessThan, version: "3.14" }]), range: RequiresPythonRange(LowerBound(Included("3.13")), UpperBound(Excluded("3.14"))) })
    DEBUG Adding direct dependency: project*
    DEBUG Solving split (markers: python_full_version == '3.14.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.14" }, VersionSpecifier { operator: LessThan, version: "3.15" }]), range: RequiresPythonRange(LowerBound(Included("3.14")), UpperBound(Excluded("3.15"))) })
    DEBUG Adding direct dependency: project*
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (*)
    DEBUG Adding direct dependency: anchor{python_full_version == '3.12.*'}*
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (*)
    DEBUG Adding direct dependency: fluctuating{python_full_version == '3.13.*'}*
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (*)
    DEBUG Adding direct dependency: consumer-delay-000{python_full_version == '3.14.*'}*
    DEBUG Searching for a compatible version of anchor{python_full_version == '3.12.*'} (*)
    DEBUG Selecting: anchor==1.0.0 [compatible] (anchor-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for anchor==1.0.0: anchor==1.0.0
    DEBUG Adding transitive dependency for anchor==1.0.0: anchor{python_full_version == '3.12.*'}==1.0.0
    DEBUG Searching for a compatible version of fluctuating{python_full_version == '3.13.*'} (*)
    DEBUG Selecting: fluctuating==1.0.0 [compatible] (fluctuating-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for fluctuating==1.0.0: fluctuating==1.0.0
    DEBUG Adding transitive dependency for fluctuating==1.0.0: fluctuating{python_full_version == '3.13.*'}==1.0.0
    DEBUG Searching for a compatible version of consumer-delay-000{python_full_version == '3.14.*'} (*)
    DEBUG Selecting: consumer-delay-000==1.0.0 [compatible] (consumer_delay_000-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-000==1.0.0
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-000{python_full_version == '3.14.*'}==1.0.0
    DEBUG Searching for a compatible version of anchor (==1.0.0)
    DEBUG Selecting: anchor==1.0.0 [compatible] (anchor-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for anchor==1.0.0: shared>=2.0.0, <2.0.0+
    DEBUG Searching for a compatible version of fluctuating (==1.0.0)
    DEBUG Selecting: fluctuating==1.0.0 [compatible] (fluctuating-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for fluctuating==1.0.0: conflict-one*
    DEBUG Adding transitive dependency for fluctuating==1.0.0: flaky>=1.0.0, <=2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-000 (==1.0.0)
    DEBUG Selecting: consumer-delay-000==1.0.0 [compatible] (consumer_delay_000-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-001*
    DEBUG Searching for a compatible version of anchor{python_full_version == '3.12.*'} (==1.0.0)
    DEBUG Selecting: anchor==1.0.0 [compatible] (anchor-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for anchor==1.0.0: shared>=2.0.0, <2.0.0+
    DEBUG Searching for a compatible version of fluctuating{python_full_version == '3.13.*'} (==1.0.0)
    DEBUG Selecting: fluctuating==1.0.0 [compatible] (fluctuating-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for fluctuating==1.0.0: conflict-one*
    DEBUG Adding transitive dependency for fluctuating==1.0.0: flaky>=1.0.0, <=2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-000{python_full_version == '3.14.*'} (==1.0.0)
    DEBUG Selecting: consumer-delay-000==1.0.0 [compatible] (consumer_delay_000-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-001*
    DEBUG Searching for a compatible version of shared (>=2.0.0, <2.0.0+)
    DEBUG Selecting: shared==2.0.0 [compatible] (shared-2.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of conflict-one (*)
    DEBUG Selecting: conflict-one==1.0.0 [compatible] (conflict_one-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-one==1.0.0: conflict-two*
    DEBUG Searching for a compatible version of consumer-delay-001 (*)
    DEBUG Selecting: consumer-delay-001==1.0.0 [compatible] (consumer_delay_001-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-001==1.0.0: consumer-delay-002*
    DEBUG Tried 3 versions: anchor 1, project 1, shared 1
    DEBUG split `python_full_version == '3.12.*'` resolution took [TIME]
    DEBUG Searching for a compatible version of flaky (>=1.0.0, <=2.0.0+)
    DEBUG Selecting: flaky==2.0.0 [compatible] (flaky-2.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for flaky==2.0.0: leaf>=1.0.0, <1.0.0+
    DEBUG Adding transitive dependency for flaky==2.0.0: shared>=2.0.0, <2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-002 (*)
    DEBUG Selecting: consumer-delay-002==1.0.0 [compatible] (consumer_delay_002-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-002==1.0.0: consumer-delay-003*
    DEBUG Searching for a compatible version of leaf (>=1.0.0, <1.0.0+)
    DEBUG Selecting: leaf==1.0.0 [compatible] (leaf-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-003 (*)
    DEBUG Selecting: consumer-delay-003==1.0.0 [compatible] (consumer_delay_003-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-003==1.0.0: consumer-delay-004*
    DEBUG Searching for a compatible version of shared (>=2.0.0, <2.0.0+)
    DEBUG Selecting: shared==2.0.0 [preference] (shared-2.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-004 (*)
    DEBUG Selecting: consumer-delay-004==1.0.0 [compatible] (consumer_delay_004-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-004==1.0.0: consumer-delay-005*
    DEBUG Searching for a compatible version of conflict-two (*)
    DEBUG Selecting: conflict-two==1.0.0 [compatible] (conflict_two-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-two==1.0.0: conflict-three*
    DEBUG Searching for a compatible version of consumer-delay-005 (*)
    DEBUG Selecting: consumer-delay-005==1.0.0 [compatible] (consumer_delay_005-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-005==1.0.0: consumer-delay-006*
    DEBUG Searching for a compatible version of conflict-three (*)
    DEBUG Selecting: conflict-three==1.0.0 [compatible] (conflict_three-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-three==1.0.0: conflict-four*
    DEBUG Searching for a compatible version of consumer-delay-006 (*)
    DEBUG Selecting: consumer-delay-006==1.0.0 [compatible] (consumer_delay_006-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-006==1.0.0: consumer-delay-007*
    DEBUG Searching for a compatible version of conflict-four (*)
    DEBUG Selecting: conflict-four==1.0.0 [compatible] (conflict_four-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-four==1.0.0: conflict-five*
    DEBUG Searching for a compatible version of consumer-delay-007 (*)
    DEBUG Selecting: consumer-delay-007==1.0.0 [compatible] (consumer_delay_007-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-007==1.0.0: consumer-delay-008*
    DEBUG Searching for a compatible version of conflict-five (*)
    DEBUG Selecting: conflict-five==1.0.0 [compatible] (conflict_five-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-five==1.0.0: leaf>=2.0.0, <2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-008 (*)
    DEBUG Selecting: consumer-delay-008==1.0.0 [compatible] (consumer_delay_008-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-008==1.0.0: consumer-delay-009*
    DEBUG Recording unit propagation conflict of conflict-five from incompatibility of (leaf, conflict-four)
    DEBUG Recording unit propagation conflict of conflict-four from incompatibility of (leaf, conflict-three)
    DEBUG Recording unit propagation conflict of conflict-three from incompatibility of (leaf, conflict-two)
    DEBUG Recording unit propagation conflict of leaf from incompatibility of (conflict-two, flaky)
    DEBUG Recording unit propagation conflict of flaky from incompatibility of (conflict-two, flaky)
    DEBUG Package conflict-five has too many conflicts (affected), prioritizing
    DEBUG Searching for a compatible version of flaky (==1.0.0)
    DEBUG Selecting: flaky==1.0.0 [compatible] (flaky-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for flaky==1.0.0: leaf>=2.0.0, <2.0.0+
    DEBUG Adding transitive dependency for flaky==1.0.0: shared>=1.0.0, <1.0.0+
    DEBUG Searching for a compatible version of consumer-delay-009 (*)
    DEBUG Selecting: consumer-delay-009==1.0.0 [compatible] (consumer_delay_009-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-009==1.0.0: consumer-delay-010*
    DEBUG Searching for a compatible version of leaf (>=2.0.0, <2.0.0+)
    DEBUG Selecting: leaf==2.0.0 [compatible] (leaf-2.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-010 (*)
    DEBUG Selecting: consumer-delay-010==1.0.0 [compatible] (consumer_delay_010-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-010==1.0.0: consumer-delay-011*
    DEBUG Searching for a compatible version of shared (>=1.0.0, <1.0.0+)
    DEBUG Selecting: shared==1.0.0 [compatible] (shared-1.0.0-py3-none-any.whl)
    DEBUG Trying coordinated backtracking for shared==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Recording unit propagation conflict of shared from incompatibility of (anchor)
    DEBUG Recording unit propagation conflict of anchor{python_full_version == '3.12.*'} from incompatibility of (project)
    DEBUG Solving split (markers: python_full_version == '3.12.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.12" }, VersionSpecifier { operator: LessThan, version: "3.13" }]), range: RequiresPythonRange(LowerBound(Included("3.12")), UpperBound(Excluded("3.13"))) })
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (<0.1.0 | >0.1.0)
    DEBUG No compatible version found for: project
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because shared>1.0.0 cannot be used because a coordinated backtracking trial requires version 1.0.0 and all versions of anchor depend on shared==2.0.0, we can conclude that all versions of anchor cannot be used.
    And because your project depends on anchor{python_full_version < '3.13'}, we can conclude that your project's requirements are unsatisfiable.
    DEBUG Searching for a compatible version of consumer-delay-011 (*)
    DEBUG Selecting: consumer-delay-011==1.0.0 [compatible] (consumer_delay_011-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-011==1.0.0: consumer-delay-012*
    DEBUG Searching for a compatible version of conflict-two (*)
    DEBUG Selecting: conflict-two==1.0.0 [compatible] (conflict_two-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-012 (*)
    DEBUG Selecting: consumer-delay-012==1.0.0 [compatible] (consumer_delay_012-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-012==1.0.0: consumer-delay-013*
    DEBUG Searching for a compatible version of conflict-five (*)
    DEBUG Selecting: conflict-five==1.0.0 [compatible] (conflict_five-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-013 (*)
    DEBUG Selecting: consumer-delay-013==1.0.0 [compatible] (consumer_delay_013-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-013==1.0.0: consumer-delay-014*
    DEBUG Searching for a compatible version of conflict-three (*)
    DEBUG Selecting: conflict-three==1.0.0 [compatible] (conflict_three-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-014 (*)
    DEBUG Selecting: consumer-delay-014==1.0.0 [compatible] (consumer_delay_014-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-014==1.0.0: consumer-delay-015*
    DEBUG Searching for a compatible version of conflict-four (*)
    DEBUG Selecting: conflict-four==1.0.0 [compatible] (conflict_four-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-015 (*)
    DEBUG Selecting: consumer-delay-015==1.0.0 [compatible] (consumer_delay_015-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-015==1.0.0: consumer-delay-016*
    DEBUG Tried 13 versions: flaky 2, leaf 2, shared 2, conflict-five 1, conflict-four 1, conflict-one 1, conflict-three 1, conflict-two 1, fluctuating 1, project 1
    DEBUG split `python_full_version == '3.13.*'` resolution took [TIME]
    DEBUG Trying coordinated backtracking for shared==2.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.13.*'`
    DEBUG Recording unit propagation conflict of shared from incompatibility of (flaky)
    DEBUG Recording unit propagation conflict of conflict-two from incompatibility of (conflict-one, flaky)
    DEBUG Recording unit propagation conflict of flaky from incompatibility of (conflict-one, flaky)
    DEBUG Recording unit propagation conflict of conflict-one from incompatibility of (flaky, fluctuating)
    DEBUG Recording unit propagation conflict of flaky from incompatibility of (fluctuating)
    DEBUG Recording unit propagation conflict of fluctuating{python_full_version == '3.13.*'} from incompatibility of (project)
    DEBUG Solving split (markers: python_full_version == '3.13.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.13" }, VersionSpecifier { operator: LessThan, version: "3.14" }]), range: RequiresPythonRange(LowerBound(Included("3.13")), UpperBound(Excluded("3.14"))) })
    DEBUG Package shared has too many conflicts (affected), prioritizing
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (<0.1.0 | >0.1.0)
    DEBUG No compatible version found for: project
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because all versions of conflict-five depend on leaf==2.0.0 and all versions of conflict-four depend on conflict-five, we can conclude that all versions of conflict-four depend on leaf==2.0.0.
    And because all versions of conflict-three depend on conflict-four and all versions of conflict-two depend on conflict-three, we can conclude that all versions of conflict-two depend on leaf==2.0.0.
    And because flaky>=2.0.0 depends on leaf==1.0.0 and all versions of conflict-one depend on conflict-two, we can conclude that all versions of conflict-one and flaky>=2.0.0 are incompatible. (1)

    Because all of:
        shared<2.0.0
        shared>2.0.0
     cannot be used because a coordinated backtracking trial requires version 2.0.0 and flaky<=1.0.0 depends on shared==1.0.0, we can conclude that flaky<=1.0.0 cannot be used.
    And because we know from (1) that all versions of conflict-one and flaky>=2.0.0 are incompatible, we can conclude that all versions of conflict-one and all versions of flaky are incompatible.
    And because all versions of fluctuating depend on conflict-one, we can conclude that all versions of flaky and all versions of fluctuating are incompatible.
    And because all versions of fluctuating depend on flaky and your project depends on fluctuating{python_full_version == '3.13.*'}, we can conclude that your project's requirements are unsatisfiable.
    DEBUG Searching for a compatible version of consumer-delay-016 (*)
    DEBUG Selecting: consumer-delay-016==1.0.0 [compatible] (consumer_delay_016-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-016==1.0.0: consumer-delay-017*
    DEBUG Searching for a compatible version of consumer-delay-017 (*)
    DEBUG Selecting: consumer-delay-017==1.0.0 [compatible] (consumer_delay_017-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-017==1.0.0: consumer-delay-018*
    DEBUG Searching for a compatible version of consumer-delay-018 (*)
    DEBUG Selecting: consumer-delay-018==1.0.0 [compatible] (consumer_delay_018-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-018==1.0.0: consumer-delay-019*
    DEBUG Searching for a compatible version of consumer-delay-019 (*)
    DEBUG Selecting: consumer-delay-019==1.0.0 [compatible] (consumer_delay_019-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-019==1.0.0: consumer-delay-020*
    DEBUG Searching for a compatible version of consumer-delay-020 (*)
    DEBUG Selecting: consumer-delay-020==1.0.0 [compatible] (consumer_delay_020-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-020==1.0.0: consumer-delay-021*
    DEBUG Searching for a compatible version of consumer-delay-021 (*)
    DEBUG Selecting: consumer-delay-021==1.0.0 [compatible] (consumer_delay_021-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-021==1.0.0: consumer-delay-022*
    DEBUG Searching for a compatible version of consumer-delay-022 (*)
    DEBUG Selecting: consumer-delay-022==1.0.0 [compatible] (consumer_delay_022-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-022==1.0.0: consumer-delay-023*
    DEBUG Searching for a compatible version of consumer-delay-023 (*)
    DEBUG Selecting: consumer-delay-023==1.0.0 [compatible] (consumer_delay_023-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-023==1.0.0: consumer*
    DEBUG Searching for a compatible version of consumer (*)
    DEBUG Selecting: consumer==1.0.0 [compatible] (consumer-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer==1.0.0: shared>=1.0.0, <=3.0.0+
    DEBUG Searching for a compatible version of shared (>=1.0.0, <=3.0.0+)
    DEBUG Selecting: shared==2.0.0 [preference] (shared-2.0.0-py3-none-any.whl)
    DEBUG Tried 27 versions: consumer 1, consumer-delay-000 1, consumer-delay-001 1, consumer-delay-002 1, consumer-delay-003 1, consumer-delay-004 1, consumer-delay-005 1, consumer-delay-006 1, consumer-delay-007 1, consumer-delay-008 1, consumer-delay-009 1, consumer-delay-010 1, consumer-delay-011 1, consumer-delay-012 1, consumer-delay-013 1, consumer-delay-014 1, consumer-delay-015 1, consumer-delay-016 1, consumer-delay-017 1, consumer-delay-018 1, consumer-delay-019 1, consumer-delay-020 1, consumer-delay-021 1, consumer-delay-022 1, consumer-delay-023 1, project 1, shared 1
    DEBUG split `python_full_version == '3.14.*'` resolution took [TIME]
    DEBUG Trying coordinated backtracking for shared==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.14.*'`
    DEBUG Solving split (markers: python_full_version == '3.14.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.14" }, VersionSpecifier { operator: LessThan, version: "3.15" }]), range: RequiresPythonRange(LowerBound(Included("3.14")), UpperBound(Excluded("3.15"))) })
    DEBUG Searching for a compatible version of shared (==1.0.0)
    DEBUG Selecting: shared==1.0.0 [preference] (shared-1.0.0-py3-none-any.whl)
    DEBUG Tried 28 versions: shared 2, consumer 1, consumer-delay-000 1, consumer-delay-001 1, consumer-delay-002 1, consumer-delay-003 1, consumer-delay-004 1, consumer-delay-005 1, consumer-delay-006 1, consumer-delay-007 1, consumer-delay-008 1, consumer-delay-009 1, consumer-delay-010 1, consumer-delay-011 1, consumer-delay-012 1, consumer-delay-013 1, consumer-delay-014 1, consumer-delay-015 1, consumer-delay-016 1, consumer-delay-017 1, consumer-delay-018 1, consumer-delay-019 1, consumer-delay-020 1, consumer-delay-021 1, consumer-delay-022 1, consumer-delay-023 1, project 1
    DEBUG split `python_full_version == '3.14.*'` resolution took [TIME]
    DEBUG Completed coordinated backtracking fork for split `python_full_version == '3.14.*'`
    DEBUG Rejected coordinated backtracking: duplicate count 1 -> 1
    INFO Solved your requirements for 3 environments
    DEBUG Distinct solution for split (markers: python_full_version == '3.12.*') with 3 package(s)
    DEBUG Distinct solution for split (markers: python_full_version == '3.13.*') with 10 package(s)
    DEBUG Distinct solution for split (markers: python_full_version == '3.14.*') with 27 package(s)
    Resolved 37 packages in [TIME]
    "#);
    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    anchor==1.0.0 ; python_full_version < '3.13'
    conflict-five==1.0.0 ; python_full_version == '3.13.*'
    conflict-four==1.0.0 ; python_full_version == '3.13.*'
    conflict-one==1.0.0 ; python_full_version == '3.13.*'
    conflict-three==1.0.0 ; python_full_version == '3.13.*'
    conflict-two==1.0.0 ; python_full_version == '3.13.*'
    consumer==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-000==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-001==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-002==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-003==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-004==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-005==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-006==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-007==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-008==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-009==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-010==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-011==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-012==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-013==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-014==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-015==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-016==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-017==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-018==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-019==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-020==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-021==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-022==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-023==1.0.0 ; python_full_version >= '3.14'
    flaky==1.0.0 ; python_full_version == '3.13.*'
    fluctuating==1.0.0 ; python_full_version == '3.13.*'
    leaf==2.0.0 ; python_full_version == '3.13.*'
    shared==1.0.0 ; python_full_version == '3.13.*'
    shared==2.0.0 ; python_full_version != '3.13.*'
    "#);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--locked", "--offline"])
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 37 packages in [TIME]
    "#);
    assert_eq!(locked, context.read("uv.lock"));

    Ok(())
}

/// Once the last owner backtracks, its withdrawn version must no longer influence a later
/// sibling. Otherwise `highest` prefers the stale version over the remaining live observation.
#[test]
fn coordinated_observations_withdrawn_version_is_not_preferred() -> Result<()> {
    let scenario: Scenario = toml::from_str(include_str!(
        "../../../../test/scenarios/fork/coordinated-observations-withdrawn-version.toml"
    ))?;
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.15"
        dependencies = ["anchor; python_version == '3.12'", "fluctuating; python_version == '3.13'", "consumer-delay-000; python_version == '3.14'"]

        [tool.uv]
        fork-strategy = "fewest"
        environments = ["python_version == '3.12'", "python_version == '3.13'", "python_version == '3.14'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::RUST_LOG, "uv_resolver::resolver=debug")
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Solving with installed Python version: 3.12.[X]
    DEBUG Solving with target Python version: >=3.12, <3.15
    DEBUG Narrowed `requires-python` bound to: >=3.14, <3.15
    DEBUG Narrowed `requires-python` bound to: >=3.13, <3.14
    DEBUG Narrowed `requires-python` bound to: >=3.12, <3.13
    DEBUG Solving split (markers: python_full_version == '3.12.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.12" }, VersionSpecifier { operator: LessThan, version: "3.13" }]), range: RequiresPythonRange(LowerBound(Included("3.12")), UpperBound(Excluded("3.13"))) })
    DEBUG Adding direct dependency: project*
    DEBUG Solving split (markers: python_full_version == '3.13.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.13" }, VersionSpecifier { operator: LessThan, version: "3.14" }]), range: RequiresPythonRange(LowerBound(Included("3.13")), UpperBound(Excluded("3.14"))) })
    DEBUG Adding direct dependency: project*
    DEBUG Solving split (markers: python_full_version == '3.14.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.14" }, VersionSpecifier { operator: LessThan, version: "3.15" }]), range: RequiresPythonRange(LowerBound(Included("3.14")), UpperBound(Excluded("3.15"))) })
    DEBUG Adding direct dependency: project*
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (*)
    DEBUG Adding direct dependency: anchor{python_full_version == '3.12.*'}*
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (*)
    DEBUG Adding direct dependency: fluctuating{python_full_version == '3.13.*'}*
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (*)
    DEBUG Adding direct dependency: consumer-delay-000{python_full_version == '3.14.*'}*
    DEBUG Searching for a compatible version of anchor{python_full_version == '3.12.*'} (*)
    DEBUG Selecting: anchor==1.0.0 [compatible] (anchor-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for anchor==1.0.0: anchor==1.0.0
    DEBUG Adding transitive dependency for anchor==1.0.0: anchor{python_full_version == '3.12.*'}==1.0.0
    DEBUG Searching for a compatible version of fluctuating{python_full_version == '3.13.*'} (*)
    DEBUG Selecting: fluctuating==1.0.0 [compatible] (fluctuating-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for fluctuating==1.0.0: fluctuating==1.0.0
    DEBUG Adding transitive dependency for fluctuating==1.0.0: fluctuating{python_full_version == '3.13.*'}==1.0.0
    DEBUG Searching for a compatible version of consumer-delay-000{python_full_version == '3.14.*'} (*)
    DEBUG Selecting: consumer-delay-000==1.0.0 [compatible] (consumer_delay_000-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-000==1.0.0
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-000{python_full_version == '3.14.*'}==1.0.0
    DEBUG Searching for a compatible version of anchor (==1.0.0)
    DEBUG Selecting: anchor==1.0.0 [compatible] (anchor-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for anchor==1.0.0: shared>=1.0.0, <1.0.0+
    DEBUG Searching for a compatible version of fluctuating (==1.0.0)
    DEBUG Selecting: fluctuating==1.0.0 [compatible] (fluctuating-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for fluctuating==1.0.0: conflict-one*
    DEBUG Adding transitive dependency for fluctuating==1.0.0: flaky>=1.0.0, <=2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-000 (==1.0.0)
    DEBUG Selecting: consumer-delay-000==1.0.0 [compatible] (consumer_delay_000-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-001*
    DEBUG Searching for a compatible version of anchor{python_full_version == '3.12.*'} (==1.0.0)
    DEBUG Selecting: anchor==1.0.0 [compatible] (anchor-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for anchor==1.0.0: shared>=1.0.0, <1.0.0+
    DEBUG Searching for a compatible version of fluctuating{python_full_version == '3.13.*'} (==1.0.0)
    DEBUG Selecting: fluctuating==1.0.0 [compatible] (fluctuating-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for fluctuating==1.0.0: conflict-one*
    DEBUG Adding transitive dependency for fluctuating==1.0.0: flaky>=1.0.0, <=2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-000{python_full_version == '3.14.*'} (==1.0.0)
    DEBUG Selecting: consumer-delay-000==1.0.0 [compatible] (consumer_delay_000-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-000==1.0.0: consumer-delay-001*
    DEBUG Searching for a compatible version of shared (>=1.0.0, <1.0.0+)
    DEBUG Selecting: shared==1.0.0 [compatible] (shared-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of conflict-one (*)
    DEBUG Selecting: conflict-one==1.0.0 [compatible] (conflict_one-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-one==1.0.0: conflict-two*
    DEBUG Searching for a compatible version of consumer-delay-001 (*)
    DEBUG Selecting: consumer-delay-001==1.0.0 [compatible] (consumer_delay_001-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-001==1.0.0: consumer-delay-002*
    DEBUG Tried 3 versions: anchor 1, project 1, shared 1
    DEBUG split `python_full_version == '3.12.*'` resolution took [TIME]
    DEBUG Searching for a compatible version of flaky (>=1.0.0, <=2.0.0+)
    DEBUG Selecting: flaky==2.0.0 [compatible] (flaky-2.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for flaky==2.0.0: leaf>=1.0.0, <1.0.0+
    DEBUG Adding transitive dependency for flaky==2.0.0: shared>=2.0.0, <2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-002 (*)
    DEBUG Selecting: consumer-delay-002==1.0.0 [compatible] (consumer_delay_002-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-002==1.0.0: consumer-delay-003*
    DEBUG Searching for a compatible version of leaf (>=1.0.0, <1.0.0+)
    DEBUG Selecting: leaf==1.0.0 [compatible] (leaf-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-003 (*)
    DEBUG Selecting: consumer-delay-003==1.0.0 [compatible] (consumer_delay_003-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-003==1.0.0: consumer-delay-004*
    DEBUG Searching for a compatible version of shared (>=2.0.0, <2.0.0+)
    DEBUG Selecting: shared==2.0.0 [compatible] (shared-2.0.0-py3-none-any.whl)
    DEBUG Trying coordinated backtracking for shared==2.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Recording unit propagation conflict of shared from incompatibility of (anchor)
    DEBUG Recording unit propagation conflict of anchor{python_full_version == '3.12.*'} from incompatibility of (project)
    DEBUG Solving split (markers: python_full_version == '3.12.*') (requires-python: RequiresPython { specifiers: VersionSpecifiers([VersionSpecifier { operator: GreaterThanEqual, version: "3.12" }, VersionSpecifier { operator: LessThan, version: "3.13" }]), range: RequiresPythonRange(LowerBound(Included("3.12")), UpperBound(Excluded("3.13"))) })
    DEBUG Searching for a compatible version of project @ `file://[TEMP_DIR]/` (<0.1.0 | >0.1.0)
    DEBUG No compatible version found for: project
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because all of:
        shared<2.0.0
        shared>2.0.0
     cannot be used because a coordinated backtracking trial requires version 2.0.0 and all versions of anchor depend on shared==1.0.0, we can conclude that all versions of anchor cannot be used.
    And because your project depends on anchor{python_full_version < '3.13'}, we can conclude that your project's requirements are unsatisfiable.
    DEBUG Searching for a compatible version of consumer-delay-004 (*)
    DEBUG Selecting: consumer-delay-004==1.0.0 [compatible] (consumer_delay_004-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-004==1.0.0: consumer-delay-005*
    DEBUG Searching for a compatible version of conflict-two (*)
    DEBUG Selecting: conflict-two==1.0.0 [compatible] (conflict_two-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-two==1.0.0: conflict-three*
    DEBUG Searching for a compatible version of consumer-delay-005 (*)
    DEBUG Selecting: consumer-delay-005==1.0.0 [compatible] (consumer_delay_005-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-005==1.0.0: consumer-delay-006*
    DEBUG Searching for a compatible version of conflict-three (*)
    DEBUG Selecting: conflict-three==1.0.0 [compatible] (conflict_three-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-three==1.0.0: conflict-four*
    DEBUG Searching for a compatible version of consumer-delay-006 (*)
    DEBUG Selecting: consumer-delay-006==1.0.0 [compatible] (consumer_delay_006-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-006==1.0.0: consumer-delay-007*
    DEBUG Searching for a compatible version of conflict-four (*)
    DEBUG Selecting: conflict-four==1.0.0 [compatible] (conflict_four-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-four==1.0.0: conflict-five*
    DEBUG Searching for a compatible version of consumer-delay-007 (*)
    DEBUG Selecting: consumer-delay-007==1.0.0 [compatible] (consumer_delay_007-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-007==1.0.0: consumer-delay-008*
    DEBUG Searching for a compatible version of conflict-five (*)
    DEBUG Selecting: conflict-five==1.0.0 [compatible] (conflict_five-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for conflict-five==1.0.0: leaf>=2.0.0, <2.0.0+
    DEBUG Searching for a compatible version of consumer-delay-008 (*)
    DEBUG Selecting: consumer-delay-008==1.0.0 [compatible] (consumer_delay_008-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-008==1.0.0: consumer-delay-009*
    DEBUG Recording unit propagation conflict of conflict-five from incompatibility of (leaf, conflict-four)
    DEBUG Recording unit propagation conflict of conflict-four from incompatibility of (leaf, conflict-three)
    DEBUG Recording unit propagation conflict of conflict-three from incompatibility of (leaf, conflict-two)
    DEBUG Recording unit propagation conflict of leaf from incompatibility of (conflict-two, flaky)
    DEBUG Recording unit propagation conflict of flaky from incompatibility of (conflict-two, flaky)
    DEBUG Package conflict-five has too many conflicts (affected), prioritizing
    DEBUG Searching for a compatible version of flaky (==1.0.0)
    DEBUG Selecting: flaky==1.0.0 [compatible] (flaky-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for flaky==1.0.0: leaf>=2.0.0, <2.0.0+
    DEBUG Adding transitive dependency for flaky==1.0.0: shared>=1.0.0, <1.0.0+
    DEBUG Searching for a compatible version of consumer-delay-009 (*)
    DEBUG Selecting: consumer-delay-009==1.0.0 [compatible] (consumer_delay_009-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-009==1.0.0: consumer-delay-010*
    DEBUG Searching for a compatible version of leaf (>=2.0.0, <2.0.0+)
    DEBUG Selecting: leaf==2.0.0 [compatible] (leaf-2.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-010 (*)
    DEBUG Selecting: consumer-delay-010==1.0.0 [compatible] (consumer_delay_010-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-010==1.0.0: consumer-delay-011*
    DEBUG Searching for a compatible version of shared (>=1.0.0, <1.0.0+)
    DEBUG Selecting: shared==1.0.0 [preference] (shared-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-011 (*)
    DEBUG Selecting: consumer-delay-011==1.0.0 [compatible] (consumer_delay_011-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-011==1.0.0: consumer-delay-012*
    DEBUG Searching for a compatible version of conflict-two (*)
    DEBUG Selecting: conflict-two==1.0.0 [compatible] (conflict_two-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-012 (*)
    DEBUG Selecting: consumer-delay-012==1.0.0 [compatible] (consumer_delay_012-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-012==1.0.0: consumer-delay-013*
    DEBUG Searching for a compatible version of conflict-five (*)
    DEBUG Selecting: conflict-five==1.0.0 [compatible] (conflict_five-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-013 (*)
    DEBUG Selecting: consumer-delay-013==1.0.0 [compatible] (consumer_delay_013-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-013==1.0.0: consumer-delay-014*
    DEBUG Searching for a compatible version of conflict-three (*)
    DEBUG Selecting: conflict-three==1.0.0 [compatible] (conflict_three-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-014 (*)
    DEBUG Selecting: consumer-delay-014==1.0.0 [compatible] (consumer_delay_014-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-014==1.0.0: consumer-delay-015*
    DEBUG Searching for a compatible version of conflict-four (*)
    DEBUG Selecting: conflict-four==1.0.0 [compatible] (conflict_four-1.0.0-py3-none-any.whl)
    DEBUG Searching for a compatible version of consumer-delay-015 (*)
    DEBUG Selecting: consumer-delay-015==1.0.0 [compatible] (consumer_delay_015-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-015==1.0.0: consumer-delay-016*
    DEBUG Tried 13 versions: flaky 2, leaf 2, shared 2, conflict-five 1, conflict-four 1, conflict-one 1, conflict-three 1, conflict-two 1, fluctuating 1, project 1
    DEBUG split `python_full_version == '3.13.*'` resolution took [TIME]
    DEBUG Searching for a compatible version of consumer-delay-016 (*)
    DEBUG Selecting: consumer-delay-016==1.0.0 [compatible] (consumer_delay_016-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-016==1.0.0: consumer-delay-017*
    DEBUG Searching for a compatible version of consumer-delay-017 (*)
    DEBUG Selecting: consumer-delay-017==1.0.0 [compatible] (consumer_delay_017-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-017==1.0.0: consumer-delay-018*
    DEBUG Searching for a compatible version of consumer-delay-018 (*)
    DEBUG Selecting: consumer-delay-018==1.0.0 [compatible] (consumer_delay_018-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-018==1.0.0: consumer-delay-019*
    DEBUG Searching for a compatible version of consumer-delay-019 (*)
    DEBUG Selecting: consumer-delay-019==1.0.0 [compatible] (consumer_delay_019-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-019==1.0.0: consumer-delay-020*
    DEBUG Searching for a compatible version of consumer-delay-020 (*)
    DEBUG Selecting: consumer-delay-020==1.0.0 [compatible] (consumer_delay_020-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-020==1.0.0: consumer-delay-021*
    DEBUG Searching for a compatible version of consumer-delay-021 (*)
    DEBUG Selecting: consumer-delay-021==1.0.0 [compatible] (consumer_delay_021-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-021==1.0.0: consumer-delay-022*
    DEBUG Searching for a compatible version of consumer-delay-022 (*)
    DEBUG Selecting: consumer-delay-022==1.0.0 [compatible] (consumer_delay_022-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-022==1.0.0: consumer-delay-023*
    DEBUG Searching for a compatible version of consumer-delay-023 (*)
    DEBUG Selecting: consumer-delay-023==1.0.0 [compatible] (consumer_delay_023-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer-delay-023==1.0.0: consumer*
    DEBUG Searching for a compatible version of consumer (*)
    DEBUG Selecting: consumer==1.0.0 [compatible] (consumer-1.0.0-py3-none-any.whl)
    DEBUG Adding transitive dependency for consumer==1.0.0: shared>=1.0.0, <=3.0.0+
    DEBUG Searching for a compatible version of shared (>=1.0.0, <=3.0.0+)
    DEBUG Selecting: shared==1.0.0 [preference] (shared-1.0.0-py3-none-any.whl)
    DEBUG Tried 27 versions: consumer 1, consumer-delay-000 1, consumer-delay-001 1, consumer-delay-002 1, consumer-delay-003 1, consumer-delay-004 1, consumer-delay-005 1, consumer-delay-006 1, consumer-delay-007 1, consumer-delay-008 1, consumer-delay-009 1, consumer-delay-010 1, consumer-delay-011 1, consumer-delay-012 1, consumer-delay-013 1, consumer-delay-014 1, consumer-delay-015 1, consumer-delay-016 1, consumer-delay-017 1, consumer-delay-018 1, consumer-delay-019 1, consumer-delay-020 1, consumer-delay-021 1, consumer-delay-022 1, consumer-delay-023 1, project 1, shared 1
    DEBUG split `python_full_version == '3.14.*'` resolution took [TIME]
    INFO Solved your requirements for 3 environments
    DEBUG Distinct solution for split (markers: python_full_version == '3.12.*') with 3 package(s)
    DEBUG Distinct solution for split (markers: python_full_version == '3.13.*') with 10 package(s)
    DEBUG Distinct solution for split (markers: python_full_version == '3.14.*') with 27 package(s)
    Resolved 36 packages in [TIME]
    "#);
    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    anchor==1.0.0 ; python_full_version < '3.13'
    conflict-five==1.0.0 ; python_full_version == '3.13.*'
    conflict-four==1.0.0 ; python_full_version == '3.13.*'
    conflict-one==1.0.0 ; python_full_version == '3.13.*'
    conflict-three==1.0.0 ; python_full_version == '3.13.*'
    conflict-two==1.0.0 ; python_full_version == '3.13.*'
    consumer==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-000==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-001==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-002==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-003==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-004==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-005==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-006==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-007==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-008==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-009==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-010==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-011==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-012==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-013==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-014==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-015==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-016==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-017==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-018==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-019==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-020==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-021==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-022==1.0.0 ; python_full_version >= '3.14'
    consumer-delay-023==1.0.0 ; python_full_version >= '3.14'
    flaky==1.0.0 ; python_full_version == '3.13.*'
    fluctuating==1.0.0 ; python_full_version == '3.13.*'
    leaf==2.0.0 ; python_full_version == '3.13.*'
    shared==1.0.0
    "#);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--locked", "--offline"])
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 36 packages in [TIME]
    "#);
    assert_eq!(locked, context.read("uv.lock"));
    Ok(())
}

//! Connector suite for entropy, against the operating system and the stub.

use connectors::Random;
use connectors_host::OsRandom;
use pretty_assertions::assert_eq;
use testkit::StubRandom;

fn with_each_random(check: fn(&dyn Random)) {
    check(&OsRandom);
    check(&StubRandom::default());
}

#[test]
fn as_many_bytes_as_asked_for_are_returned() {
    with_each_random(|random| {
        assert_eq!([random.bytes(0).len(), random.bytes(33).len()], [0, 33]);
    });
}

#[test]
fn two_draws_from_the_operating_system_differ() {
    let random = OsRandom;

    let first = random.bytes(32);
    let second = random.bytes(32);

    assert_ne!(first, second);
}

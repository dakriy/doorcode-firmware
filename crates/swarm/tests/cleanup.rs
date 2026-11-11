//! Connector suite for deleting what a load run created for good: the calls
//! made to the server, and what is reported kept when the server refuses
//! the force, does not know it, or fails, from scripted answers.

use pretty_assertions::assert_eq;
use swarm::cleanup::{delete_for_good, Answer};

/// Deletes `/doors/12` for good against a server that gives `answers` in
/// turn; returns what is reported kept and the calls made.
fn delete_door(answers: Vec<Answer>) -> (Option<String>, Vec<String>) {
    let mut answers = answers.into_iter();
    let mut calls = Vec::new();
    let kept = delete_for_good("/doors/12", |method, path| {
        calls.push(format!("{method} {path}"));
        answers.next().expect("an answer for every call")
    });
    (kept, calls)
}

fn status(code: u16, body: &str) -> Answer {
    Ok((code, body.to_owned()))
}

#[test]
fn a_door_deleted_for_good_and_then_not_found_is_gone() {
    let (kept, calls) = delete_door(vec![
        status(204, ""),
        status(404, "{\"error\":\"not found\"}"),
    ]);

    assert_eq!(kept, None);
    assert_eq!(calls, ["DELETE /doors/12?force=true", "GET /doors/12"]);
}

#[test]
fn a_door_still_found_after_deleting_for_good_is_kept_by_a_server_that_does_not_know_force() {
    let (kept, _) = delete_door(vec![status(204, ""), status(200, "{\"id\":12}")]);

    assert_eq!(
        kept.as_deref(),
        Some(
            "/doors/12 is deleted but kept: the server still has it after deleting it for good, \
             so it does not know force=true and deleted it the ordinary way"
        )
    );
}

#[test]
fn a_force_refused_to_a_token_without_admin_deletes_the_ordinary_way_and_says_why() {
    let (kept, calls) = delete_door(vec![
        status(403, "{\"error\":\"forbidden\"}"),
        status(204, ""),
    ]);

    assert_eq!(
        kept.as_deref(),
        Some(
            "/doors/12 is deleted but kept: the server answered 403 {\"error\":\"forbidden\"} \
             (only a token of a user holding admin may) to deleting it for good, so it is \
             deleted the ordinary way"
        )
    );
    assert_eq!(calls, ["DELETE /doors/12?force=true", "DELETE /doors/12"]);
}

#[test]
fn a_force_refused_for_an_override_or_firmware_deletes_the_ordinary_way_and_says_why() {
    let (kept, _) = delete_door(vec![
        status(409, "{\"error\":\"overrides\"}"),
        status(204, ""),
    ]);

    assert_eq!(
        kept.as_deref(),
        Some(
            "/doors/12 is deleted but kept: the server answered 409 {\"error\":\"overrides\"} \
             (the user made an override or uploaded firmware that still exists) to deleting it \
             for good, so it is deleted the ordinary way"
        )
    );
}

#[test]
fn a_refused_force_followed_by_a_failed_ordinary_delete_is_not_deleted() {
    let (kept, _) = delete_door(vec![status(403, ""), status(500, "{\"error\":\"failed\"}")]);

    assert_eq!(
        kept.as_deref(),
        Some(
            "/doors/12 is not deleted: the server answered 403 (only a token of a user holding \
             admin may) to deleting it for good, and 500 {\"error\":\"failed\"} to deleting it \
             the ordinary way"
        )
    );
}

#[test]
fn a_force_the_server_fails_is_not_deleted() {
    let (kept, calls) = delete_door(vec![status(500, "{\"error\":\"failed\"}")]);

    assert_eq!(
        kept.as_deref(),
        Some(
            "/doors/12 is not deleted: the server answered 500 {\"error\":\"failed\"} to \
             deleting it for good"
        )
    );
    assert_eq!(calls, ["DELETE /doors/12?force=true"]);
}

#[test]
fn a_force_nothing_answers_is_not_deleted() {
    let (kept, _) = delete_door(vec![Err("connection refused".to_owned())]);

    assert_eq!(
        kept.as_deref(),
        Some(
            "/doors/12 is not deleted: the server answered nothing (connection refused) to \
             deleting it for good"
        )
    );
}

#[test]
fn a_door_the_check_after_deleting_for_good_cannot_find_out_about_may_be_kept() {
    let (kept, _) = delete_door(vec![status(204, ""), status(500, "{\"error\":\"failed\"}")]);

    assert_eq!(
        kept.as_deref(),
        Some(
            "/doors/12 may be kept: the server answered 500 {\"error\":\"failed\"} when asked \
             whether it is gone"
        )
    );
}

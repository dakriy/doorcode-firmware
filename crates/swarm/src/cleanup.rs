//! Deleting a door or user the run created for good, and saying what the
//! server kept when it would not.

/// What the server answered a call: its status and the start of its body,
/// or why nothing answered.
pub type Answer = Result<(u16, String), String>;

/// Deletes the door or user at `path`, such as `/doors/12`, for good with
/// `DELETE path?force=true` through `call(method, path)`, and checks that
/// `GET path` then finds nothing. Returns `None` when it is gone, and
/// otherwise a line saying what is kept and why. When the server refuses
/// to force, with 403 to a token whose user does not hold `admin` or 409
/// for a user who made an override or uploaded firmware, it deletes `path`
/// the ordinary way instead, so it is still refused everywhere. A server
/// too old to know `force` deletes it the ordinary way, which the check
/// finds.
pub fn delete_for_good(path: &str, mut call: impl FnMut(&str, &str) -> Answer) -> Option<String> {
    let forced = call("DELETE", &format!("{path}?force=true"));
    match forced {
        Ok((200..=299, _)) => gone(path, &call("GET", path)),
        Ok((403 | 409, _)) => Some(refused(path, &forced, &call("DELETE", path))),
        _ => Some(format!(
            "{path} is not deleted: the server answered {} to deleting it for good",
            said(&forced)
        )),
    }
}

/// What `found`, the answer to asking for `path` after deleting it for
/// good, says is kept.
fn gone(path: &str, found: &Answer) -> Option<String> {
    match found {
        Ok((404, _)) => None,
        Ok((200..=299, _)) => Some(format!(
            "{path} is deleted but kept: the server still has it after deleting it for good, \
             so it does not know force=true and deleted it the ordinary way"
        )),
        _ => Some(format!(
            "{path} may be kept: the server answered {} when asked whether it is gone",
            said(found)
        )),
    }
}

/// What is kept after the server answered `forced` to deleting `path` for
/// good and `plain` to deleting it the ordinary way.
fn refused(path: &str, forced: &Answer, plain: &Answer) -> String {
    let refusal = format!("the server answered {}{}", said(forced), why(forced));
    if matches!(plain, Ok((200..=299, _))) {
        return format!(
            "{path} is deleted but kept: {refusal} to deleting it for good, \
             so it is deleted the ordinary way"
        );
    }
    format!(
        "{path} is not deleted: {refusal} to deleting it for good, and {} to deleting it \
         the ordinary way",
        said(plain)
    )
}

/// Why the server refuses to delete for good with the status `forced`.
fn why(forced: &Answer) -> &'static str {
    match forced {
        Ok((403, _)) => " (only a token of a user holding admin may)",
        _ => " (the user made an override or uploaded firmware that still exists)",
    }
}

fn said(answer: &Answer) -> String {
    match answer {
        Ok((status, body)) => format!("{status} {body}").trim_end().to_owned(),
        Err(error) => format!("nothing ({error})"),
    }
}

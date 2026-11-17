use nagi_model::ObjectId;

pub(crate) fn valid_web_url(url: &str) -> bool {
    let Some(authority_and_path) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    !authority.is_empty()
        && !url.chars().any(char::is_control)
        && !url.chars().any(char::is_whitespace)
}

pub(crate) fn object_id_from_uri(target: &str) -> Option<ObjectId> {
    let value = target.strip_prefix("nagi-object://")?;
    if value.len() != 16 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(value, 16).ok().map(ObjectId)
}

pub(crate) fn valid_image_target(target: &str) -> bool {
    valid_web_url(target) || object_id_from_uri(target).is_some()
}

use txbase::edge::ObjectStoreError;

pub(crate) const LOCK_FILE: &str = ".txbase-object-store.lock";
pub(crate) const TEMP_PREFIX: &str = ".txbase-object-store-tmp-";

pub(crate) fn validate_key(key: &str) -> Result<(), ObjectStoreError> {
    if key.is_empty() || key.contains('\0') {
        return Err(ObjectStoreError::Invalid(
            "object key must be non-empty and must not contain NUL".into(),
        ));
    }

    for component in key.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component == LOCK_FILE
            || component.starts_with(TEMP_PREFIX)
            || component.ends_with([' ', '.'])
            || component.chars().any(|character| {
                matches!(character, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            })
        {
            return Err(ObjectStoreError::Invalid(format!(
                "object key component is not portable: {component}"
            )));
        }
    }

    Ok(())
}

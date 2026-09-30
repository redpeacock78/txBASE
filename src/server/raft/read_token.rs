#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    Invalid,
    ClusterMismatch,
}

pub(super) fn encode(cluster_id: &str, applied_index: u64) -> String {
    format!("v1.{cluster_id}.{applied_index}")
}

pub(super) fn parse(value: &str, cluster_id: &str) -> Result<u64, Error> {
    let Some(value) = value.strip_prefix("v1.") else {
        return Err(Error::Invalid);
    };
    let Some((token_cluster, index)) = value.rsplit_once('.') else {
        return Err(Error::Invalid);
    };
    if token_cluster != cluster_id {
        return Err(Error::ClusterMismatch);
    }
    index.parse().map_err(|_| Error::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trip_preserves_cluster_and_applied_index() {
        let token = encode("test.cluster-1", 42);
        assert_eq!(token, "v1.test.cluster-1.42");
        assert_eq!(parse(&token, "test.cluster-1"), Ok(42));
    }

    #[test]
    fn token_rejects_invalid_version_cluster_and_index() {
        assert_eq!(parse("v2.cluster.42", "cluster"), Err(Error::Invalid));
        assert_eq!(parse("v1.other.42", "cluster"), Err(Error::ClusterMismatch));
        assert_eq!(parse("v1.cluster.nope", "cluster"), Err(Error::Invalid));
        assert_eq!(parse("v1.cluster.42.extra", "cluster"), Err(Error::Invalid));
    }
}

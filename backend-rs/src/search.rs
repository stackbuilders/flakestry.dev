use opensearch::{
    http::StatusCode,
    indices::{IndicesCreateParts, IndicesGetParts},
    OpenSearch, SearchParts,
};
use serde_json::{json, Value};

pub async fn create_flake_index(opensearch: &OpenSearch) -> Result<(), opensearch::Error> {
    let status = opensearch
        .indices()
        .get(IndicesGetParts::Index(&["flakes"]))
        .send()
        .await?
        .status_code();

    if status == StatusCode::NOT_FOUND {
        let _ = opensearch
            .indices()
            .create(IndicesCreateParts::Index("flakes"))
            .send()
            .await?;
    }

    Ok(())
}

pub async fn search_flakes(
    opensearch: &OpenSearch,
    q: &String,
) -> Result<Value, opensearch::Error> {
    let res = opensearch
        .search(SearchParts::Index(&["flakes"]))
        .size(10)
        .body(json!({
            "query": {
                "multi_match": {
                    "query": q,
                    "fuzziness": "AUTO",
                    "fields": [
                        "description^2",
                        "readme",
                        "outputs",
                        "repo^2",
                        "owner^2",
                    ],
                }
            }
        }))
        .send()
        .await?
        .json::<Value>()
        .await?;

    Ok(res)
}

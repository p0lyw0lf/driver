use serde::Deserialize;
use serde::Serialize;

use crate::engine::{Producer, QueryContext, db::Object, db::remote::Uri};
use crate::query_key;

query_key!(GetUrl(pub Uri););

impl Producer for GetUrl {
    type Output = crate::Result<Object>;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        Ok(ctx
            .db()
            .remotes
            .fetch(&ctx.db().objects, self.0.clone())
            .await?
            .object)
    }
}

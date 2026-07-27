use axum::{routing::get, Router};
use sqlx::PgPool;

async fn hello() -> &'static str { "Hello, world!" }
async fn full_stack(pool: axum::extract::State<PgPool>) -> String {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM polls_question WHERE pub_date <= now() ORDER BY pub_date DESC LIMIT 5",
    ).fetch_all(&*pool).await.unwrap();
    format!("<ul>{}</ul>", rows.iter().map(|(id,)| format!("<li><a href=\"/{id}/\">{id}</a></li>")).collect::<String>())
}

#[tokio::main]
async fn main() {
    let pool = PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
    let app = Router::new().route("/hello/", get(hello)).route("/", get(full_stack)).with_state(pool);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9001").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

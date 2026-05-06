use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

const AKRA_OFFICE_BACKGROUND: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/akra-office-background.png");
const AKRA_OBJECT_SPRITES: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/akra-object-sprites.png");

pub(super) async fn admin_graphic_asset(
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let bytes = match asset_name.as_str() {
        "akra-office-background.png" => AKRA_OFFICE_BACKGROUND,
        "akra-object-sprites.png" => AKRA_OBJECT_SPRITES,
        _ => return Err(StatusCode::NOT_FOUND),
    };

    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

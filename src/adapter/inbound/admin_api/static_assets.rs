use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

const AKRA_OFFICE_BACKGROUND: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/akra-office-background.png");
const AKRA_OBJECT_SPRITES: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/akra-object-sprites.png");
const GAMEBALJEONGUK_ATLAS_64X96: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/gamebaljeonguk_atlas_64x96.png");

// Individual isometric sprites for PixiJS diorama
const SPRITE_FLOOR_TILE: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_floor_tile.png");
const SPRITE_DESK_WORKSTATION: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_desk_workstation.png");
const SPRITE_SERVER_RACK: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_server_rack.png");
const SPRITE_WHITEBOARD: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_whiteboard.png");
const SPRITE_SOFA: &[u8] = include_bytes!("../../../../assets/admin/graphics/sprite_sofa.png");
const SPRITE_POTTED_PLANT: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_potted_plant.png");
const AKRA_DIORAMA_JS: &[u8] = include_bytes!("../../../../assets/admin/game/akra-diorama.js");

pub(super) async fn admin_graphic_asset(
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let bytes = match asset_name.as_str() {
        "akra-office-background.png" => AKRA_OFFICE_BACKGROUND,
        "akra-object-sprites.png" => AKRA_OBJECT_SPRITES,
        "gamebaljeonguk_atlas_64x96.png" => GAMEBALJEONGUK_ATLAS_64X96,
        "sprite_floor_tile.png" => SPRITE_FLOOR_TILE,
        "sprite_desk_workstation.png" => SPRITE_DESK_WORKSTATION,
        "sprite_server_rack.png" => SPRITE_SERVER_RACK,
        "sprite_whiteboard.png" => SPRITE_WHITEBOARD,
        "sprite_sofa.png" => SPRITE_SOFA,
        "sprite_potted_plant.png" => SPRITE_POTTED_PLANT,
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

pub(super) async fn admin_game_asset(
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let (content_type, bytes) = match asset_name.as_str() {
        "akra-diorama.js" => ("text/javascript; charset=utf-8", AKRA_DIORAMA_JS),
        _ => return Err(StatusCode::NOT_FOUND),
    };

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

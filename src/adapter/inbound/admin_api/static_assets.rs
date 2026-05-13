use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

const AKRA_OFFICE_BACKGROUND: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/akra-office-background.png");
const FINAL_DRAFT_MAP_SPRITE: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/final-draft-map-sprite.png");
const AKRA_OBJECT_SPRITES: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/akra-object-sprites.png");
const GAMEBALJEONGUK_ATLAS_64X96: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/gamebaljeonguk_atlas_64x96.png");
const GAMEBALJEONGUK_ATLAS_128X192: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/gamebaljeonguk_atlas_128x192.png");

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
const SPRITE_FD_DESK_1: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_desk_1.png");
const SPRITE_FD_DESK_2: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_desk_2.png");
const SPRITE_FD_DESK_3: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_desk_3.png");
const SPRITE_FD_DESK_4: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_desk_4.png");
const SPRITE_FD_DESK_5: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_desk_5.png");
const SPRITE_FD_BOSS_DESK: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_boss_desk.png");
const SPRITE_FD_DISTRIBUTOR_DESK: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_distributor_desk.png");
const SPRITE_FD_EVENT_LOG_TOWER: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_event_log_tower.png");
const SPRITE_FD_SOFA: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_sofa.png");
const SPRITE_FD_POTTED_PLANT: &[u8] =
    include_bytes!("../../../../assets/admin/graphics/sprite_fd_potted_plant.png");
const AKRA_DIORAMA_JS: &[u8] = include_bytes!("../../../../assets/admin/game/akra-diorama.js");

pub(super) async fn admin_graphic_asset(
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let bytes = match asset_name.as_str() {
        "akra-office-background.png" => AKRA_OFFICE_BACKGROUND,
        "final-draft-map-sprite.png" => FINAL_DRAFT_MAP_SPRITE,
        "akra-object-sprites.png" => AKRA_OBJECT_SPRITES,
        "gamebaljeonguk_atlas_64x96.png" => GAMEBALJEONGUK_ATLAS_64X96,
        "gamebaljeonguk_atlas_128x192.png" => GAMEBALJEONGUK_ATLAS_128X192,
        "sprite_floor_tile.png" => SPRITE_FLOOR_TILE,
        "sprite_desk_workstation.png" => SPRITE_DESK_WORKSTATION,
        "sprite_server_rack.png" => SPRITE_SERVER_RACK,
        "sprite_whiteboard.png" => SPRITE_WHITEBOARD,
        "sprite_sofa.png" => SPRITE_SOFA,
        "sprite_potted_plant.png" => SPRITE_POTTED_PLANT,
        "sprite_fd_desk_1.png" => SPRITE_FD_DESK_1,
        "sprite_fd_desk_2.png" => SPRITE_FD_DESK_2,
        "sprite_fd_desk_3.png" => SPRITE_FD_DESK_3,
        "sprite_fd_desk_4.png" => SPRITE_FD_DESK_4,
        "sprite_fd_desk_5.png" => SPRITE_FD_DESK_5,
        "sprite_fd_boss_desk.png" => SPRITE_FD_BOSS_DESK,
        "sprite_fd_distributor_desk.png" => SPRITE_FD_DISTRIBUTOR_DESK,
        "sprite_fd_event_log_tower.png" => SPRITE_FD_EVENT_LOG_TOWER,
        "sprite_fd_sofa.png" => SPRITE_FD_SOFA,
        "sprite_fd_potted_plant.png" => SPRITE_FD_POTTED_PLANT,
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

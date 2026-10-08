use super::*;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use diesel::connection::SimpleConnection;
use serde_json::{Value, json};
use tower::ServiceExt;

fn setup() -> (AppState, User, User, User) {
    let manager = diesel::r2d2::ConnectionManager::<SqliteConnection>::new(":memory:");
    let pool = diesel::r2d2::Pool::builder()
        .max_size(1)
        .build(manager)
        .unwrap();
    let mut conn = pool.get().unwrap();
    conn.batch_execute("PRAGMA foreign_keys = ON;").unwrap();
    for sql in [
        include_str!("../../migrations/2026-10-01-080305-0000_create_file_meta/up.sql"),
        include_str!("../../migrations/2026-10-01-121330-0000_add_parent_and_is_dir/up.sql"),
        include_str!("../../migrations/2026-10-01-122204-0000_add_is_link_and_target/up.sql"),
        include_str!("../../migrations/2026-10-01-135407-0000_create_users/up.sql"),
        include_str!("../../migrations/2026-10-04-000000-0000_remove_link_fields/up.sql"),
        include_str!("../../migrations/2026-10-06-000000-0000_create_shares/up.sql"),
        include_str!("../../migrations/2026-10-07-000000-0000_add_admin_role/up.sql"),
        include_str!("../../migrations/2026-10-08-000000-0000_add_share_owner/up.sql"),
    ] {
        conn.batch_execute(sql).unwrap();
    }
    crate::file_system::file_manager::ensure_root(&mut conn).unwrap();
    let admin = crate::user::ensure_admin(&mut conn, "admin-test-password")
        .unwrap()
        .unwrap();
    let alice = UserManager::create_user(&mut conn, "alice".into(), "alice-test-password").unwrap();
    let bob = UserManager::create_user(&mut conn, "bob".into(), "bob-test-password").unwrap();
    drop(conn);
    (AppState { db: pool }, admin, alice, bob)
}

async fn call(
    state: &AppState,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = router(state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let text = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    (status, text)
}

async fn directory(state: &AppState, token: &str, owner: Uuid, parent: Uuid) -> Uuid {
    let (status, text) = call(state, Some(token), "POST", "/files", json!({"file_name":"folder", "file_hash":"", "is_directory":true, "file_owner":owner, "parent_id":parent})).await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    let value: Value = serde_json::from_str(&text).unwrap();
    Uuid::parse_str(value["file_id"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn ordinary_users_cannot_manage_accounts_or_foreign_files() {
    let (state, admin, alice, bob) = setup();
    let admin_token = crate::jwt::generate_token(&admin).unwrap();
    let token = crate::jwt::generate_token(&alice).unwrap();
    let folder = directory(&state, &admin_token, bob.user_id, ROOT_ID).await;
    for (method, path, body) in [
        ("GET", "/admin/users".into(), json!(null)),
        ("GET", format!("/admin/users/{}", bob.user_id), json!(null)),
        (
            "DELETE",
            format!("/admin/users/{}", bob.user_id),
            json!(null),
        ),
        (
            "PATCH",
            format!("/admin/users/{}", alice.user_id),
            json!({"is_admin":true}),
        ),
        (
            "GET",
            format!("/files?user_id={}", bob.user_id),
            json!(null),
        ),
        ("GET", format!("/files/{folder}"), json!(null)),
        (
            "PATCH",
            format!("/files/{folder}"),
            json!({"file_name":"hacked"}),
        ),
        ("DELETE", format!("/files/{folder}"), json!(null)),
        ("GET", format!("/download/{folder}"), json!(null)),
        (
            "POST",
            "/files".into(),
            json!({"file_name":"hacked", "file_hash":"", "is_directory":true, "file_owner":bob.user_id}),
        ),
        ("POST", "/move".into(), json!({"file_id":folder})),
        (
            "POST",
            "/create_share".into(),
            json!({"file_id_list":[folder]}),
        ),
    ] {
        assert_eq!(
            call(&state, Some(&token), method, &path, body).await.0,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }
    let request = Request::builder().method("POST").uri(format!("/upload?user_id={}", bob.user_id))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "multipart/form-data; boundary=test-boundary")
        .body(Body::from("--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\n\r\ntest\r\n--test-boundary--\r\n")).unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&state, None, "GET", "/admin/users", json!(null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, text) = call(
        &state,
        None,
        "POST",
        "/register",
        json!({"username":"new-user", "password":"test-password", "is_admin":true}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(user["is_admin"], false);
    assert!(user.get("password_hash").is_none());
}

#[tokio::test]
async fn deleting_a_user_removes_files_shares_and_login_access() {
    let (state, admin, alice, bob) = setup();
    let token = crate::jwt::generate_token(&admin).unwrap();
    let alice_token = crate::jwt::generate_token(&alice).unwrap();
    let folder = directory(&state, &token, alice.user_id, ROOT_ID).await;
    let child = directory(&state, &token, alice.user_id, folder).await;
    let other = directory(&state, &token, bob.user_id, ROOT_ID).await;
    let (owned_share, mixed_share) = {
        let mut conn = state.db.get().unwrap();
        let own = ShareManager::create_share(&mut conn, alice.user_id, vec![child], None).unwrap();
        let mixed =
            ShareManager::create_share_as(&mut conn, admin.user_id, vec![child, other], None, true)
                .unwrap();
        (own.share_code, mixed.share_code)
    };
    assert_eq!(
        call(
            &state,
            Some(&token),
            "DELETE",
            &format!("/admin/users/{}", alice.user_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&state, Some(&alice_token), "GET", "/user_info", json!(null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            None,
            "POST",
            "/login",
            json!({"username":"alice", "password":"alice-test-password"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "GET",
            &format!("/admin/users/{}", alice.user_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "DELETE",
            &format!("/admin/users/{}", alice.user_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut conn = state.db.get().unwrap();
    let remaining = crate::schema::file_meta::table
        .load::<FileMeta>(&mut conn)
        .unwrap();
    assert!(
        !remaining
            .iter()
            .any(|file| file.file_owner == alice.user_id)
    );
    assert!(remaining.iter().any(|file| file.file_id == other));
    assert!(
        ShareManager::get_share(&mut conn, &owned_share)
            .unwrap()
            .is_none()
    );
    let files = ShareManager::get_files(&mut conn, &mixed_share).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_id, other);
}

#[tokio::test]
async fn user_deletion_is_atomic_and_protects_admin_accounts() {
    let (state, admin, alice, _) = setup();
    let token = crate::jwt::generate_token(&admin).unwrap();
    assert_eq!(
        call(
            &state,
            Some(&token),
            "DELETE",
            &format!("/admin/users/{}", admin.user_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let folder = directory(&state, &token, alice.user_id, ROOT_ID).await;
    let code = {
        let mut conn = state.db.get().unwrap();
        let share =
            ShareManager::create_share(&mut conn, alice.user_id, vec![folder], None).unwrap();
        conn.batch_execute("CREATE TRIGGER reject_user_delete BEFORE DELETE ON users BEGIN SELECT RAISE(ABORT, 'simulated deletion failure'); END;").unwrap();
        share.share_code
    };
    assert_eq!(
        call(
            &state,
            Some(&token),
            "DELETE",
            &format!("/admin/users/{}", alice.user_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let mut conn = state.db.get().unwrap();
    assert!(
        UserManager::get_user_by_id(&mut conn, alice.user_id)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        ShareManager::get_files(&mut conn, &code).unwrap()[0].file_id,
        folder
    );
    // Independently enforce the last-admin invariant in the transaction helper.
    let mut actor = alice.clone();
    actor.is_admin = true;
    assert_eq!(
        delete_user_records(&mut conn, &actor, admin.user_id)
            .err()
            .unwrap()
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn blob_cleanup_preserves_content_referenced_by_other_users() {
    let (state, admin, alice, bob) = setup();
    let store = std::env::temp_dir().join(format!("netdisk-delete-test-{}", Uuid::new_v4()));
    std::fs::create_dir(&store).unwrap();
    let shared_hash = blake3::hash(b"shared").to_hex().to_string();
    let private_hash = blake3::hash(b"private").to_hex().to_string();
    std::fs::write(store.join(&shared_hash), b"shared").unwrap();
    std::fs::write(store.join(&private_hash), b"private").unwrap();
    let mut conn = state.db.get().unwrap();
    for (owner, hash) in [
        (alice.user_id, &shared_hash),
        (bob.user_id, &shared_hash),
        (alice.user_id, &private_hash),
    ] {
        diesel::insert_into(crate::schema::file_meta::table)
            .values(FileMeta {
                file_id: Uuid::new_v4(),
                file_name: "test.txt".into(),
                file_size: 6,
                file_hash: hash.clone(),
                file_owner: owner,
                file_created_at: 0,
                file_updated_at: 0,
                parent_id: Uuid::new_v4(),
                is_directory: false,
            })
            .execute(&mut conn)
            .unwrap();
    }
    let hashes = delete_user_records(&mut conn, &admin, alice.user_id).unwrap();
    cleanup_user_blobs(&mut conn, &hashes, &store);
    assert_eq!(std::fs::read(store.join(&shared_hash)).unwrap(), b"shared");
    assert!(!store.join(&private_hash).exists());
    assert_eq!(
        crate::schema::file_meta::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        2
    ); // root + Bob's file
    std::fs::remove_dir_all(store).unwrap();
}

#[tokio::test]
async fn admin_manages_foreign_folders_but_preserves_owner_and_root() {
    let (state, admin, alice, bob) = setup();
    let token = crate::jwt::generate_token(&admin).unwrap();
    let folder = directory(&state, &token, alice.user_id, ROOT_ID).await;
    let target = directory(&state, &token, alice.user_id, ROOT_ID).await;
    let foreign_target = directory(&state, &token, bob.user_id, ROOT_ID).await;
    let (status, text) = call(&state, Some(&token), "GET", "/admin/users", json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!text.contains("password_hash"));
    assert_eq!(
        call(
            &state,
            Some(&token),
            "GET",
            &format!("/files?user_id={}", alice.user_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "PATCH",
            &format!("/files/{folder}"),
            json!({"file_name":"renamed"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "POST",
            "/move",
            json!({"file_id":folder, "new_parent_id":foreign_target})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "POST",
            "/move",
            json!({"file_id":folder, "new_parent_id":target})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "POST",
            "/move",
            json!({"file_id":target, "new_parent_id":folder})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "POST",
            "/create_share",
            json!({"file_id_list":[folder]})
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "GET",
            &format!("/download/{target}"),
            json!(null)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "DELETE",
            &format!("/files/{ROOT_ID}"),
            json!(null)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "DELETE",
            &format!("/files/{target}"),
            json!(null)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "GET",
            &format!("/files/{folder}"),
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn account_changes_revoke_password_sessions_and_apply_roles_immediately() {
    let (state, admin, alice, _) = setup();
    let token = crate::jwt::generate_token(&admin).unwrap();
    let alice_token = crate::jwt::generate_token(&alice).unwrap();
    assert_eq!(
        call(
            &state,
            Some(&token),
            "PATCH",
            &format!("/admin/users/{}", admin.user_id),
            json!({"is_admin":false})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let path = format!("/admin/users/{}", alice.user_id);
    assert_eq!(
        call(
            &state,
            Some(&token),
            "PATCH",
            &path,
            json!({"is_admin":true})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            Some(&alice_token),
            "GET",
            "/admin/users",
            json!(null)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            Some(&token),
            "PATCH",
            &path,
            json!({"is_admin":false, "username":"renamed-user", "password":"changed-password"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, Some(&alice_token), "GET", "/user_info", json!(null))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            None,
            "POST",
            "/login",
            json!({"username":"alice", "password":"alice-test-password"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, new_token) = call(
        &state,
        None,
        "POST",
        "/login",
        json!({"username":"renamed-user", "password":"changed-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(&state, Some(&new_token), "GET", "/admin/users", json!(null))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut conn = state.db.get().unwrap();
    assert!(
        crate::user::ensure_admin(&mut conn, "another-password")
            .unwrap()
            .is_none()
    );
    assert!(
        UserManager::get_user_by_id(&mut conn, admin.user_id)
            .unwrap()
            .unwrap()
            .verify_password("admin-test-password")
    );
    conn.batch_execute(include_str!(
        "../../migrations/2026-10-07-000000-0000_add_admin_role/down.sql"
    ))
    .unwrap();
    conn.batch_execute(include_str!(
        "../../migrations/2026-10-07-000000-0000_add_admin_role/up.sql"
    ))
    .unwrap();
}

#[tokio::test]
async fn share_lists_are_creator_scoped_and_revocation_keeps_source_files() {
    let (state, admin, alice, bob) = setup();
    let admin_token = crate::jwt::generate_token(&admin).unwrap();
    let alice_token = crate::jwt::generate_token(&alice).unwrap();
    let bob_token = crate::jwt::generate_token(&bob).unwrap();
    let folder = directory(&state, &alice_token, alice.user_id, ROOT_ID).await;
    let (own, delegated) = {
        let mut conn = state.db.get().unwrap();
        (
            ShareManager::create_share(&mut conn, alice.user_id, vec![folder], None).unwrap(),
            ShareManager::create_share_as(&mut conn, admin.user_id, vec![folder], None, true)
                .unwrap(),
        )
    };
    let (status, text) = call(
        &state,
        Some(&alice_token),
        "GET",
        "/share_records",
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let records: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(records.as_array().unwrap().len(), 1);
    assert_eq!(records[0]["share_id"], own.dic_id.to_string());
    assert_eq!(records[0]["names"][0], "folder");
    for path in [
        format!("/share_records?user_id={}", alice.user_id),
        "/share_records?all=true".into(),
    ] {
        assert_eq!(
            call(&state, Some(&bob_token), "GET", &path, json!(null))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&state, Some(&admin_token), "GET", &path, json!(null))
                .await
                .0,
            StatusCode::OK
        );
    }
    let path = format!("/share_records/{}", own.dic_id);
    assert_eq!(
        call(&state, Some(&bob_token), "DELETE", &path, json!(null))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            Some(&alice_token),
            "DELETE",
            &format!("/share_records/{}", delegated.dic_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&state, Some(&alice_token), "DELETE", &path, json!(null))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &state,
            Some(&bob_token),
            "GET",
            &format!("/shares/{}", own.share_code),
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &state,
            Some(&bob_token),
            "GET",
            &format!("/shares/{}/download/{folder}", own.share_code),
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &state,
            Some(&alice_token),
            "GET",
            &format!("/files/{folder}"),
            json!(null)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            Some(&admin_token),
            "DELETE",
            &format!("/share_records/{}", delegated.dic_id),
            json!(null)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&state, Some(&alice_token), "DELETE", &path, json!(null))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn share_owner_migration_and_recycled_codes_use_stable_ids() {
    let (state, admin, alice, bob) = setup();
    let mut conn = state.db.get().unwrap();
    // Insert source metadata directly to exercise pre-migration ownership inference.
    let mut ids = Vec::new();
    for owner in [alice.user_id, bob.user_id] {
        let id = Uuid::new_v4();
        ids.push(id);
        diesel::insert_into(crate::schema::file_meta::table)
            .values(FileMeta {
                file_id: id,
                file_name: "test".into(),
                file_size: 0,
                file_hash: "".into(),
                file_owner: owner,
                file_created_at: 0,
                file_updated_at: 0,
                parent_id: ROOT_ID,
                is_directory: true,
            })
            .execute(&mut conn)
            .unwrap();
    }
    let single = ShareManager::create_share(&mut conn, alice.user_id, vec![ids[0]], None).unwrap();
    let mixed = ShareManager::create_share_as(&mut conn, admin.user_id, ids, None, true).unwrap();
    conn.batch_execute(include_str!(
        "../../migrations/2026-10-08-000000-0000_add_share_owner/down.sql"
    ))
    .unwrap();
    conn.batch_execute(include_str!(
        "../../migrations/2026-10-08-000000-0000_add_share_owner/up.sql"
    ))
    .unwrap();
    assert_eq!(
        ShareManager::get_share(&mut conn, &single.share_code)
            .unwrap()
            .unwrap()
            .owner_id,
        Some(alice.user_id.to_string())
    );
    assert_eq!(
        ShareManager::get_share(&mut conn, &mixed.share_code)
            .unwrap()
            .unwrap()
            .owner_id,
        None
    );
    assert!(matches!(
        ShareManager::revoke(&mut conn, mixed.dic_id, alice.user_id, false),
        Err(ShareError::Forbidden)
    ));
    ShareManager::revoke(&mut conn, mixed.dic_id, admin.user_id, true).unwrap();
    let mut replacement = mixed.clone();
    replacement.dic_id = Uuid::new_v4();
    replacement.owner_id = Some(bob.user_id.to_string());
    diesel::insert_into(crate::schema::share_table::table)
        .values(replacement.clone())
        .execute(&mut conn)
        .unwrap();
    assert!(matches!(
        ShareManager::revoke(&mut conn, mixed.dic_id, admin.user_id, true),
        Err(ShareError::NotFound)
    ));
    assert_eq!(
        ShareManager::get_share(&mut conn, &mixed.share_code)
            .unwrap()
            .unwrap()
            .dic_id,
        replacement.dic_id
    );
}

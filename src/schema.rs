// @generated automatically by Diesel CLI.

diesel::table! {
    file_meta (file_id) {
        file_id -> Text,
        file_name -> Text,
        file_size -> BigInt,
        file_hash -> Text,
        file_owner -> Text,
        file_created_at -> BigInt,
        file_updated_at -> BigInt,
        parent_id -> Text,
        is_directory -> Bool,
    }
}

diesel::table! {
    share_files (dic_id, file_id) {
        dic_id -> Text,
        file_id -> Text,
    }
}

diesel::table! {
    share_table (share_code) {
        share_code -> Text,
        dic_id -> Text,
        created_at -> Timestamp,
        expired_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    users (user_id) {
        user_id -> Text,
        username -> Text,
        password_hash -> Text,
        created_at -> BigInt,
        updated_at -> BigInt,
        is_admin -> Bool,
        token_version -> BigInt,
    }
}

diesel::joinable!(share_files -> file_meta (file_id));

diesel::allow_tables_to_appear_in_same_query!(file_meta, share_files, share_table, users,);

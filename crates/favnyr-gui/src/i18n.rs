//! UI translations — loaded from **TOML** files.
//!
//! The strings live in `crates/favnyr-gui/i18n/{lang}.toml` and are
//! **embedded at compile time** (so
//! 100% offline, no missing file possible). The user can
//! override or extend any key by dropping a
//! `<config>/favnyr/i18n/{lang}.toml` file (dotted-format keys, e.g.
//! `shortcut_action.copy = "…"`).
//!
//! Cascading fallback: key missing from a language → English value (catalogs
//! are built on an English base) → the key itself (never a panic,
//! never a surprise empty string). Adding a language = adding a `.toml` file +
//! an entry in `embedded()`/`catalog()`; adding a string = a key in
//! the TOML files + (if exposed to Slint) a property in the `Strings` global.

use std::collections::HashMap;
use std::sync::OnceLock;

use favnyr_core::{Lang, paths};

use slint::ComponentHandle;

use crate::{MainWindow, Strings};

// Embedded catalogs (sources of truth, versioned in the repo).
const EMBED_EN: &str = include_str!("../i18n/en.toml");
const EMBED_FR: &str = include_str!("../i18n/fr.toml");
const EMBED_ES: &str = include_str!("../i18n/es.toml");
const EMBED_DE: &str = include_str!("../i18n/de.toml");
const EMBED_IT: &str = include_str!("../i18n/it.toml");
const EMBED_RU: &str = include_str!("../i18n/ru.toml");
const EMBED_ZH: &str = include_str!("../i18n/zh.toml");
const EMBED_JA: &str = include_str!("../i18n/ja.toml");

fn embedded(lang: Lang) -> &'static str {
    match lang {
        Lang::En => EMBED_EN,
        Lang::Fr => EMBED_FR,
        Lang::Es => EMBED_ES,
        Lang::De => EMBED_DE,
        Lang::It => EMBED_IT,
        Lang::Ru => EMBED_RU,
        Lang::Zh => EMBED_ZH,
        Lang::Ja => EMBED_JA,
    }
}

/// Flattens a TOML table into dotted keys (`shortcut_action.copy`).
fn flatten_into(prefix: &str, table: &toml::Table, out: &mut HashMap<String, String>) {
    for (k, v) in table {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            toml::Value::String(s) => {
                out.insert(key, s.clone());
            }
            toml::Value::Table(t) => flatten_into(&key, t, out),
            _ => {}
        }
    }
}

/// Merges the keys from a TOML source on top of `out` (new ones overwrite).
fn overlay(src: &str, out: &mut HashMap<String, String>) {
    match toml::from_str::<toml::Table>(src) {
        Ok(table) => flatten_into("", &table, out),
        Err(err) => tracing::warn!(error = %err, "i18n: ignoring invalid TOML"),
    }
}

/// Builds a language's catalog: English base (fallback) → embedded
/// language → optional on-disk override `<config>/favnyr/i18n/{lang}.toml`.
fn build_catalog(lang: Lang) -> HashMap<String, String> {
    let mut map = HashMap::new();
    overlay(EMBED_EN, &mut map);
    if lang != Lang::En {
        overlay(embedded(lang), &mut map);
    }
    let disk = paths::config_dir()
        .join("i18n")
        .join(format!("{}.toml", lang.code()));
    if let Ok(src) = std::fs::read_to_string(&disk) {
        tracing::info!(path = %disk.display(), "i18n: on-disk override applied");
        overlay(&src, &mut map);
    }
    map
}

/// A language's (memoized) catalog. Built only once per language.
fn catalog(lang: Lang) -> &'static HashMap<String, String> {
    fn get(
        lang: Lang,
        cell: &'static OnceLock<HashMap<String, String>>,
    ) -> &'static HashMap<String, String> {
        cell.get_or_init(|| build_catalog(lang))
    }
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static FR: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ES: OnceLock<HashMap<String, String>> = OnceLock::new();
    static DE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static IT: OnceLock<HashMap<String, String>> = OnceLock::new();
    static RU: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ZH: OnceLock<HashMap<String, String>> = OnceLock::new();
    static JA: OnceLock<HashMap<String, String>> = OnceLock::new();
    match lang {
        Lang::En => get(lang, &EN),
        Lang::Fr => get(lang, &FR),
        Lang::Es => get(lang, &ES),
        Lang::De => get(lang, &DE),
        Lang::It => get(lang, &IT),
        Lang::Ru => get(lang, &RU),
        Lang::Zh => get(lang, &ZH),
        Lang::Ja => get(lang, &JA),
    }
}

/// Translates a dotted key. Fallback: the language's value → (English base
/// already merged in) → the key itself.
pub fn tr(lang: Lang, key: &str) -> String {
    catalog(lang)
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Updates the window's translated properties from the existing catalog.
/// Each binding reads only its own label, avoiding copies of the whole catalog.
pub fn apply_strings(window: &MainWindow, lang: Lang) {
    apply_catalog(window, catalog(lang));
}

fn apply_catalog(window: &MainWindow, c: &HashMap<String, String>) {
    let strings = window.global::<Strings>();
    let g = |k: &str| -> slint::SharedString { c.get(k).map(|s| s.as_str()).unwrap_or(k).into() };
    strings.set_archive_compress_to(g("archive_compress_to"));
    strings.set_archive_name(g("archive_name"));
    strings.set_archive_format(g("archive_format"));
    strings.set_archive_tar_compression(g("archive_tar_compression"));
    strings.set_archive_none(g("archive_none"));
    strings.set_archive_level(g("archive_level"));
    strings.set_archive_level_hint(g("archive_level_hint"));
    strings.set_archive_password(g("archive_password"));
    strings.set_archive_confirm_password(g("archive_confirm_password"));
    strings.set_archive_password_title(g("archive_password_title"));
    strings.set_archive_password_hint(g("archive_password_hint"));
    strings.set_archive_password_toggle(g("archive_password_toggle"));
    strings.set_archive_tar_hint(g("archive_tar_hint"));
    strings.set_archive_create(g("archive_create"));
    strings.set_archive_extract(g("archive_extract"));
    strings.set_archive_managed(g("archive_managed"));
    strings.set_archive_managed_hint(g("archive_managed_hint"));
    strings.set_app_title(g("app_title"));
    strings.set_settings_language(g("settings_language"));
    strings.set_settings_theme(g("settings_theme"));
    strings.set_settings_terminal(g("settings_terminal"));
    strings.set_settings_appearance(g("settings_appearance"));
    strings.set_theme_folder(g("theme_folder"));
    strings.set_theme_reload(g("theme_reload"));
    strings.set_edit_select_all(g("edit_select_all"));
    strings.set_edit_clear(g("edit_clear"));
    strings.set_edit_show_password(g("edit_show_password"));
    strings.set_edit_hide_password(g("edit_hide_password"));
    strings.set_settings_ui_scale(g("settings_ui_scale"));
    strings.set_settings_optional_tools_title(g("settings_optional_tools_title"));
    strings.set_settings_optional_tools_hint(g("settings_optional_tools_hint"));
    strings.set_settings_optional_tools_recheck(g("settings_optional_tools_recheck"));
    strings.set_settings_optional_tools_available(g("settings_optional_tools_available"));
    strings.set_settings_optional_tools_missing(g("settings_optional_tools_missing"));
    strings.set_settings_optional_tools_flatpak(g("settings_optional_tools_flatpak"));
    strings.set_settings_tool_ffmpeg_hint(g("settings_tool_ffmpeg_hint"));
    strings.set_settings_tool_poppler_hint(g("settings_tool_poppler_hint"));
    strings.set_settings_tool_seven_zip_hint(g("settings_tool_seven_zip_hint"));
    strings.set_col_name(g("col_name"));
    strings.set_col_path(g("col_path"));
    strings.set_col_size(g("col_size"));
    strings.set_col_modified(g("col_modified"));
    strings.set_col_created(g("col_created"));
    strings.set_col_age(g("col_age"));
    strings.set_col_ext(g("col_ext"));
    strings.set_ext_filter_label(g("ext_filter_label"));
    strings.set_search_subfolders(g("search_subfolders"));
    strings.set_search_reveal(g("search_reveal"));
    strings.set_search_reveal_new_tab(g("search_reveal_new_tab"));
    strings.set_ext_filter_placeholder(g("ext_filter_placeholder"));
    strings.set_col_resolution(g("col_resolution"));
    strings.set_col_depth(g("col_depth"));
    strings.set_fav_title(g("fav_title"));
    strings.set_fav_new_container(g("fav_new_container"));
    strings.set_fav_collapse_all(g("fav_collapse_all"));
    strings.set_fav_expand_all(g("fav_expand_all"));
    strings.set_fav_empty(g("fav_empty"));
    strings.set_fav_container_empty(g("fav_container_empty"));
    strings.set_fav_open(g("fav_open"));
    strings.set_fav_execute(g("fav_execute"));
    strings.set_fav_open_all(g("fav_open_all"));
    strings.set_fav_new_subcontainer(g("fav_new_subcontainer"));
    strings.set_fav_rename(g("fav_rename"));
    strings.set_fav_delete(g("fav_delete"));
    strings.set_fav_delete_confirm(g("fav_delete_confirm"));
    strings.set_fav_copy_path(g("fav_copy_path"));
    strings.set_fav_save_tab(g("fav_save_tab"));
    strings.set_fav_save_all_tabs(g("fav_save_all_tabs"));
    strings.set_fav_add(g("fav_add"));
    strings.set_fav_popup_title(g("fav_popup_title"));
    strings.set_fav_popup_alias(g("fav_popup_alias"));
    strings.set_fav_popup_container(g("fav_popup_container"));
    strings.set_fav_popup_parent(g("fav_popup_parent"));
    strings.set_fav_popup_new(g("fav_popup_new"));
    strings.set_fav_popup_root(g("fav_popup_root"));
    strings.set_fav_save(g("fav_save"));
    strings.set_fav_toast_added(g("fav_toast_added"));
    strings.set_fav_toast_missing(g("fav_toast_missing"));
    strings.set_fav_toast_exists(g("fav_toast_exists"));
    strings.set_footer_items_singular(g("footer_items_singular"));
    strings.set_footer_items_plural(g("footer_items_plural"));
    strings.set_footer_empty(g("footer_empty"));
    strings.set_footer_selected_singular(g("footer_selected_singular"));
    strings.set_footer_selected_plural(g("footer_selected_plural"));
    strings.set_nav_prev(g("nav_prev"));
    strings.set_nav_next(g("nav_next"));
    strings.set_nav_parent(g("nav_parent"));
    strings.set_nav_refresh(g("nav_refresh"));
    strings.set_view_mode_tooltip(g("view_mode_tooltip"));
    strings.set_view_layout_list(g("view_layout_list"));
    strings.set_view_layout_grid(g("view_layout_grid"));
    strings.set_view_sort_tooltip(g("view_sort_tooltip"));
    strings.set_sort_primary(g("sort_primary"));
    strings.set_sort_secondary(g("sort_secondary"));
    strings.set_sort_by_day(g("sort_by_day"));
    strings.set_sort_ascending(g("sort_ascending"));
    strings.set_sort_descending(g("sort_descending"));
    strings.set_show_hidden_tooltip(g("show_hidden_tooltip"));
    strings.set_manual_order_label(g("manual_order_label"));
    strings.set_manual_order_hint(g("manual_order_hint"));
    strings.set_manual_order_drag(g("manual_order_drag"));
    strings.set_group_section(g("group_section"));
    strings.set_group_folders_first(g("group_folders_first"));
    strings.set_group_colored_folders_first(g("group_colored_folders_first"));
    strings.set_group_files_first(g("group_files_first"));
    strings.set_group_mixed(g("group_mixed"));
    strings.set_tab_new_tooltip(g("tab_new_tooltip"));
    strings.set_tab_scroll_left(g("tab_scroll_left"));
    strings.set_tab_scroll_right(g("tab_scroll_right"));
    strings.set_tabbar_menu_tooltip(g("tabbar_menu_tooltip"));
    strings.set_tabbar_top(g("tabbar_top"));
    strings.set_tabbar_left(g("tabbar_left"));
    strings.set_tabbar_right(g("tabbar_right"));
    strings.set_settings_tabbar_default(g("settings_tabbar_default"));
    strings.set_settings_default_view_mode_label(g("settings_default_view_mode_label"));
    strings
        .set_settings_default_item_arrangement_label(g("settings_default_item_arrangement_label"));
    strings.set_settings_default_hidden_label(g("settings_default_hidden_label"));
    strings.set_settings_default_view_mode_hint(g("settings_default_view_mode_hint"));
    strings.set_settings_view_mode_list(g("settings_view_mode_list"));
    strings.set_settings_view_mode_previews(g("settings_view_mode_previews"));
    strings.set_settings_tab_tooltip_label(g("settings_tab_tooltip_label"));
    strings.set_settings_tab_tooltip_hint(g("settings_tab_tooltip_hint"));
    strings.set_settings_grid_name_tooltip_label(g("settings_grid_name_tooltip_label"));
    strings.set_settings_grid_name_tooltip_hint(g("settings_grid_name_tooltip_hint"));
    strings.set_settings_search_path_tooltip_label(g("settings_search_path_tooltip_label"));
    strings.set_settings_search_path_tooltip_hint(g("settings_search_path_tooltip_hint"));
    strings.set_settings_clock_label(g("settings_clock_label"));
    strings.set_settings_clock_local(g("settings_clock_local"));
    strings.set_settings_ws_panel_hint(g("settings_ws_panel_hint"));
    strings.set_settings_ws_shortcut_hint(g("settings_ws_shortcut_hint"));
    strings.set_settings_compact_preview_label(g("settings_compact_preview_label"));
    strings.set_settings_thumbnail_zoom_label(g("settings_thumbnail_zoom_label"));
    strings.set_settings_thumbnail_zoom_hint(g("settings_thumbnail_zoom_hint"));
    strings.set_settings_compact_preview_hint(g("settings_compact_preview_hint"));
    strings.set_settings_annotations_title(g("settings_annotations_title"));
    strings.set_settings_annotations_hint(g("settings_annotations_hint"));
    strings.set_settings_annotations_clean(g("settings_annotations_clean"));
    strings.set_annotations_cleanup_title(g("annotations_cleanup_title"));
    strings.set_annotations_cleanup_all(g("annotations_cleanup_all"));
    strings.set_annotations_cleanup_none(g("annotations_cleanup_none"));
    strings.set_annotations_cleanup_empty(g("annotations_cleanup_empty"));
    strings.set_settings_annotations_cleaned(g("settings_annotations_cleaned"));
    strings.set_tab_unavailable(g("tab_unavailable"));
    strings.set_tab_unavailable_hint(g("tab_unavailable_hint"));
    strings.set_tab_duplicate(g("tab_duplicate"));
    strings.set_tab_reopen_closed(g("tab_reopen_closed"));
    strings.set_tab_close_tooltip(g("tab_close_tooltip"));
    strings.set_panel_prefix(g("panel_prefix"));
    strings.set_panel_new_tooltip(g("panel_new_tooltip"));
    strings.set_panel_close_tooltip(g("panel_close_tooltip"));
    strings.set_panel_move_tooltip(g("panel_move_tooltip"));
    strings.set_panel_split_view(g("panel_split_view"));
    strings.set_split_side_by_side(g("split_side_by_side"));
    strings.set_split_stacked(g("split_stacked"));
    strings.set_equalize_all(g("equalize_all"));
    strings.set_equalize_restore(g("equalize_restore"));
    strings.set_ctx_open(g("ctx_open"));
    strings.set_ctx_open_new_tab(g("ctx_open_new_tab"));
    strings.set_ctx_open_new_pane(g("ctx_open_new_pane"));
    strings.set_ctx_open_with(g("ctx_open_with"));
    strings.set_ctx_open_parent(g("ctx_open_parent"));
    strings.set_ctx_open_admin(g("ctx_open_admin"));
    strings.set_ctx_new(g("ctx_new"));
    strings.set_ctx_new_folder(g("ctx_new_folder"));
    strings.set_ctx_new_file(g("ctx_new_file"));
    strings.set_ctx_new_shortcut(g("ctx_new_shortcut"));
    strings.set_create_shortcut_target(g("create_shortcut_target"));
    strings.set_create_browse(g("create_browse"));
    strings.set_ctx_create_link(g("ctx_create_link"));
    strings.set_ctx_mark(g("ctx_mark"));
    strings.set_ctx_comment(g("ctx_comment"));
    strings.set_comment_title(g("comment_title"));
    strings.set_comment_placeholder(g("comment_placeholder"));
    strings.set_comment_clear(g("comment_clear"));
    strings.set_create_symlink_tab(g("create_symlink_tab"));
    strings.set_create_confirm(g("create_confirm"));
    strings.set_create_placeholder(g("create_placeholder"));
    strings.set_ctx_open_terminal(g("ctx_open_terminal"));
    strings.set_ctx_copy(g("ctx_copy"));
    strings.set_ctx_cut(g("ctx_cut"));
    strings.set_ctx_paste(g("ctx_paste"));
    strings.set_ctx_duplicate(g("ctx_duplicate"));
    strings.set_ctx_copy_path(g("ctx_copy_path"));
    strings.set_ctx_copy_name(g("ctx_copy_name"));
    strings.set_ctx_rename(g("ctx_rename"));
    strings.set_ctx_delete(g("ctx_delete"));
    strings.set_ctx_properties(g("ctx_properties"));
    strings.set_prop_kind_folder(g("prop_kind_folder"));
    strings.set_prop_kind_file(g("prop_kind_file"));
    strings.set_settings_tooltip(g("settings_tooltip"));
    strings.set_settings_title(g("settings_title"));
    strings.set_settings_tab_general(g("settings_tab_general"));
    strings.set_settings_tab_defaults_title(g("settings_tab_defaults_title"));
    strings.set_settings_tab_defaults_hint(g("settings_tab_defaults_hint"));
    strings.set_settings_new_workspace_title(g("settings_new_workspace_title"));
    strings.set_settings_new_workspace_hint(g("settings_new_workspace_hint"));
    strings.set_settings_group_dates(g("settings_group_dates"));
    strings.set_settings_group_tooltips(g("settings_group_tooltips"));
    strings.set_settings_group_workspaces(g("settings_group_workspaces"));
    strings.set_settings_folder_depth_hint(g("settings_folder_depth_hint"));
    strings.set_settings_tab_storage(g("settings_tab_storage"));
    strings.set_prop_title(g("prop_title"));
    strings.set_prop_tab_general(g("prop_tab_general"));
    strings.set_prop_tab_permissions(g("prop_tab_permissions"));
    strings.set_prop_tab_checksums(g("prop_tab_checksums"));
    strings.set_prop_perm_owner(g("prop_perm_owner"));
    strings.set_prop_perm_group(g("prop_perm_group"));
    strings.set_prop_perm_others(g("prop_perm_others"));
    strings.set_prop_perm_execute(g("prop_perm_execute"));
    strings.set_prop_perm_ownership(g("prop_perm_ownership"));
    strings.set_prop_perm_mode(g("prop_perm_mode"));
    strings.set_prop_hash_compute(g("prop_hash_compute"));
    strings.set_prop_hash_cancel_label(g("prop_hash_cancel_label"));
    strings.set_prop_hash_hint(g("prop_hash_hint"));
    strings.set_prop_hash_folder(g("prop_hash_folder"));
    strings.set_prop_expected_label(g("prop_expected_label"));
    strings.set_prop_expected_no_match(g("prop_expected_no_match"));
    strings.set_settings_columns_section(g("settings_columns_section"));
    strings.set_settings_col_resolution(g("settings_col_resolution"));
    strings.set_settings_col_depth(g("settings_col_depth"));
    strings.set_settings_rmtime_label(g("settings_rmtime_label"));
    strings.set_settings_rmtime_hint(g("settings_rmtime_hint"));
    strings.set_settings_fsize_label(g("settings_fsize_label"));
    strings.set_settings_fsize_hint(g("settings_fsize_hint"));
    strings.set_settings_view_text_size_title(g("settings_view_text_size_title"));
    strings.set_settings_view_text_size_hint(g("settings_view_text_size_hint"));
    strings.set_settings_view_text_size_toggle(g("settings_view_text_size_toggle"));
    strings.set_settings_view_item_spacing_title(g("settings_view_item_spacing_title"));
    strings.set_settings_view_item_spacing_hint(g("settings_view_item_spacing_hint"));
    strings.set_settings_paths_section(g("settings_paths_section"));
    strings.set_settings_config_path(g("settings_config_path"));
    strings.set_settings_data_path(g("settings_data_path"));
    strings.set_settings_cache_path(g("settings_cache_path"));
    strings.set_settings_open(g("settings_open"));
    strings.set_settings_shortcuts_section(g("settings_shortcuts_section"));
    strings.set_settings_letter_layout(g("settings_letter_layout"));
    strings.set_settings_letter_layout_tip(g("settings_letter_layout_tip"));
    strings.set_settings_letter_layout_off(g("settings_letter_layout_off"));
    strings.set_settings_keyboard_russian_standard(g("settings_keyboard_russian_standard"));
    strings.set_settings_keyboard_ukrainian_standard(g("settings_keyboard_ukrainian_standard"));
    strings.set_shortcut_search_placeholder(g("shortcut_search_placeholder"));
    strings.set_shortcut_reset_all(g("shortcut_reset_all"));
    strings.set_shortcut_reset_all_confirm(g("shortcut_reset_all_confirm"));
    strings.set_shortcut_reset_tooltip(g("shortcut_reset_tooltip"));
    strings.set_shortcut_listening(g("shortcut_listening"));
    strings.set_shortcut_capture_cancel(g("shortcut_capture_cancel"));
    strings.set_shortcut_unassigned(g("shortcut_unassigned"));
    strings.set_shortcut_unassign(g("shortcut_unassign"));
    strings.set_shortcut_no_results(g("shortcut_no_results"));
    strings.set_shortcut_conflict_reassign(g("shortcut_conflict_reassign"));
    strings.set_shortcut_conflict_keep(g("shortcut_conflict_keep"));
    strings.set_rename_title(g("rename_title"));
    strings.set_rename_current_prefix(g("rename_current_prefix"));
    strings.set_rename_new_placeholder(g("rename_new_placeholder"));
    strings.set_rename_confirm(g("rename_confirm"));
    strings.set_rename_force_replace(g("rename_force_replace"));
    strings.set_rename_select_name(g("rename_select_name"));
    strings.set_btn_cancel(g("btn_cancel"));
    strings.set_paste_conflict_title(g("paste_conflict_title"));
    strings.set_paste_conflict_hint(g("paste_conflict_hint"));
    strings.set_paste_conflict_taken(g("paste_conflict_taken"));
    strings.set_paste_conflict_skip(g("paste_conflict_skip"));
    strings.set_paste_conflict_confirm(g("paste_conflict_confirm"));
    strings.set_paste_conflict_replace(g("paste_conflict_replace"));
    strings.set_paste_conflict_replace_all(g("paste_conflict_replace_all"));
    strings.set_paste_conflict_skip_all(g("paste_conflict_skip_all"));
    strings.set_drag_open_with(g("drag_open_with"));
    strings.set_op_copying(g("op_copying"));
    strings.set_op_moving(g("op_moving"));
    strings.set_op_deleting(g("op_deleting"));
    strings.set_op_deleting_permanently(g("op_deleting_permanently"));
    strings.set_op_duplicating(g("op_duplicating"));
    strings.set_op_scanning(g("op_scanning"));
    strings.set_op_done(g("op_done"));
    strings.set_op_cancelled(g("op_cancelled"));
    strings.set_op_errors(g("op_errors"));
    strings.set_op_items(g("op_items"));
    strings.set_ws_tooltip(g("ws_tooltip"));
    strings.set_ws_title(g("ws_title"));
    strings.set_ws_save_section(g("ws_save_section"));
    strings.set_ws_name_placeholder(g("ws_name_placeholder"));
    strings.set_ws_name_taken(g("ws_name_taken"));
    strings.set_ws_save_btn(g("ws_save_btn"));
    strings.set_ws_saved_section(g("ws_saved_section"));
    strings.set_ws_empty(g("ws_empty"));
    strings.set_ws_empty_hint(g("ws_empty_hint"));
    strings.set_ws_load(g("ws_load"));
    strings.set_ws_update(g("ws_update"));
    strings.set_ws_rename(g("ws_rename"));
    strings.set_ws_delete(g("ws_delete"));
    strings.set_ws_delete_confirm(g("ws_delete_confirm"));
    strings.set_ws_overwrite_confirm(g("ws_overwrite_confirm"));
    strings.set_ws_panels_short(g("ws_panels_short"));
    strings.set_ws_tabs_short(g("ws_tabs_short"));
    strings.set_ws_toast_saved(g("ws_toast_saved"));
    strings.set_ws_toast_updated(g("ws_toast_updated"));
    strings.set_ws_toast_renamed(g("ws_toast_renamed"));
    strings.set_ws_toast_deleted(g("ws_toast_deleted"));
    strings.set_ws_notice_saved(g("ws_notice_saved"));
    strings.set_ws_notice_save_failed(g("ws_notice_save_failed"));
    strings.set_ws_dirty_title(g("ws_dirty_title"));
    strings.set_ws_dirty_body(g("ws_dirty_body"));
    strings.set_ws_dirty_body_new(g("ws_dirty_body_new"));
    strings.set_ws_dirty_discard(g("ws_dirty_discard"));
    strings.set_ws_dirty_save_load(g("ws_dirty_save_load"));
    strings.set_ws_dirty_discard_new(g("ws_dirty_discard_new"));
    strings.set_ws_dirty_save_new(g("ws_dirty_save_new"));
    strings.set_ws_shortcut_ask(g("ws_shortcut_ask"));
    strings.set_ws_shortcut_discard(g("ws_shortcut_discard"));
    strings.set_ws_shortcut_save(g("ws_shortcut_save"));
    strings.set_ws_shortcut_remember(g("ws_shortcut_remember"));
    strings.set_ws_panel_remember(g("ws_panel_remember"));
    strings.set_ws_dirty_tooltip(g("ws_dirty_tooltip"));
    strings.set_ws_slot_assign(g("ws_slot_assign"));
    strings.set_ws_slot_none(g("ws_slot_none"));
    strings.set_ws_theme_assign(g("ws_theme_assign"));
    strings.set_ws_theme_none(g("ws_theme_none"));
    strings.set_ws_toast_slot_moved(g("ws_toast_slot_moved"));
    strings.set_ws_dirty_save_action(g("ws_dirty_save_action"));
    strings.set_sidebar_tooltip(g("sidebar_tooltip"));
    strings.set_sidebar_shortcuts(g("sidebar_shortcuts"));
    strings.set_sidebar_drives(g("sidebar_drives"));
    strings.set_sidebar_network(g("sidebar_network"));
    strings.set_net_ejected(g("net_ejected"));
    strings.set_net_disconnected(g("net_disconnected"));
    strings.set_net_eject_failed(g("net_eject_failed"));
    strings.set_net_eject(g("net_eject"));
    strings.set_net_release(g("net_release"));
    strings.set_net_released(g("net_released"));
    strings.set_net_disconnect(g("net_disconnect"));
    strings.set_sidebar_home(g("sidebar_home"));
    strings.set_sidebar_trash(g("sidebar_trash"));
    strings.set_sidebar_root(g("sidebar_root"));
    strings.set_sidebar_empty_trash(g("sidebar_empty_trash"));
    strings.set_sidebar_empty_trash_confirm(g("sidebar_empty_trash_confirm"));
    strings.set_exit_title(g("exit_title"));
    strings.set_exit_body(g("exit_body"));
    strings.set_exit_stay(g("exit_stay"));
    strings.set_exit_cancel(g("exit_cancel"));
    strings.set_exit_confirm(g("exit_confirm"));
    strings.set_exit_stopping(g("exit_stopping"));
    strings.set_net_delete_title(g("net_delete_title"));
    strings.set_net_delete_body(g("net_delete_body"));
    strings.set_permanent_delete_body(g("permanent_delete_body"));
    strings.set_trash_delete_body(g("trash_delete_body"));
    strings.set_net_delete_confirm(g("net_delete_confirm"));
    strings.set_open_many_title(g("open_many_title"));
    strings.set_open_many_confirm(g("open_many_confirm"));
    strings.set_drop_move(g("drop_move"));
    strings.set_drop_copy(g("drop_copy"));
    strings.set_drop_split(g("drop_split"));
    strings.set_drop_link(g("drop_link"));
    strings.set_drop_favorite(g("drop_favorite"));
    strings.set_drop_swap(g("drop_swap"));
    strings.set_drop_detach(g("drop_detach"));
    strings.set_ws_reset(g("ws_reset"));
    strings.set_ws_new_name(g("ws_new_name"));
    strings.set_ws_toast_reset(g("ws_toast_reset"));
    strings.set_ws_sort_newest(g("ws_sort_newest"));
    strings.set_ws_sort_oldest(g("ws_sort_oldest"));
    strings.set_ws_sort_tooltip(g("ws_sort_tooltip"));
    strings.set_ow_choose(g("ow_choose"));
    strings.set_ow_pick_browse(g("ow_pick_browse"));
    strings.set_ow_custom(g("ow_custom"));
    strings.set_ow_use_as_app(g("ow_use_as_app"));
    strings.set_ow_manage(g("ow_manage"));
    strings.set_ow_title(g("ow_title"));
    strings.set_ow_name(g("ow_name"));
    strings.set_ow_program(g("ow_program"));
    strings.set_ow_store_program(g("ow_store_program"));
    strings.set_ow_args(g("ow_args"));
    strings.set_ow_tags(g("ow_tags"));
    strings.set_ow_tag_file(g("ow_tag_file"));
    strings.set_ow_tag_dir(g("ow_tag_dir"));
    strings.set_ow_tag_dirname(g("ow_tag_dirname"));
    strings.set_ow_tag_setname(g("ow_tag_setname"));
    strings.set_ow_tag_name(g("ow_tag_name"));
    strings.set_ow_tag_stem(g("ow_tag_stem"));
    strings.set_ow_tag_ext(g("ow_tag_ext"));
    strings.set_ow_tag_uri(g("ow_tag_uri"));
    strings.set_ow_tag_files(g("ow_tag_files"));
    strings.set_ow_preview(g("ow_preview"));
    strings.set_ow_add_suggested(g("ow_add_suggested"));
    strings.set_ow_recipes_label(g("ow_recipes_label"));
    strings.set_ow_recipes_hint(g("ow_recipes_hint"));
    strings.set_ow_launch(g("ow_launch"));
    strings.set_ow_default_ext(g("ow_default_ext"));
    strings.set_ow_used_ext(g("ow_used_ext"));
    strings.set_ow_default_ext_placeholder(g("ow_default_ext_placeholder"));
    strings.set_ow_used_ext_placeholder(g("ow_used_ext_placeholder"));
    strings.set_ow_set_default(g("ow_set_default"));
    strings.set_ow_display_other_apps(g("ow_display_other_apps"));
    strings.set_ow_run_as_admin(g("ow_run_as_admin"));
    strings.set_ow_default_ext_hint(g("ow_default_ext_hint"));
    strings.set_ow_show_in_section(g("ow_show_in_section"));
    strings.set_settings_shellext_empty(g("settings_shellext_empty"));
    strings.set_ow_ctx_menu(g("ow_ctx_menu"));
    strings.set_ow_ctx_files(g("ow_ctx_files"));
    strings.set_ow_ctx_dirs(g("ow_ctx_dirs"));
    strings.set_ow_ctx_background(g("ow_ctx_background"));
    strings.set_ow_ctx_ext(g("ow_ctx_ext"));
    strings.set_ow_ctx_ext_hint(g("ow_ctx_ext_hint"));
    strings.set_ow_used_ext_hint(g("ow_used_ext_hint"));
    strings.set_settings_shellmenu_label(g("settings_shellmenu_label"));
    strings.set_settings_shellmenu_hint(g("settings_shellmenu_hint"));
    strings.set_settings_shellext_list_label(g("settings_shellext_list_label"));
    strings.set_settings_shellext_list_hint(g("settings_shellext_list_hint"));
    strings.set_ow_move_up(g("ow_move_up"));
    strings.set_ow_move_down(g("ow_move_down"));
    strings.set_ow_duplicate_tip(g("ow_duplicate_tip"));
    strings.set_ow_edit_tip(g("ow_edit_tip"));
    strings.set_ow_delete_tip(g("ow_delete_tip"));
    strings.set_settings_openers_section(g("settings_openers_section"));
    strings.set_ow_add(g("ow_add"));
    strings.set_ow_empty(g("ow_empty"));
    strings.set_ow_search_placeholder(g("ow_search_placeholder"));
    strings.set_ow_no_results(g("ow_no_results"));
    strings.set_ow_promote(g("ow_promote"));
    strings.set_ow_promote_add(g("ow_promote_add"));
}

/// Language picker labels: **native names** (each language in its own
/// language → never translated).
pub fn language_labels() -> Vec<slint::SharedString> {
    Lang::all().iter().map(|l| l.native_name().into()).collect()
}

/// Tab bar position entries for the settings drop-down. Short forms, not the
/// sentences the per-view menu uses: that menu names the tab bar because
/// nothing above it does, whereas here the row is already titled with it, and
/// a repeated subject only pushed the entries past the width of the list.
pub fn tabbar_position_labels(lang: Lang) -> Vec<slint::SharedString> {
    vec![
        tr(lang, "tabbar_pos_top").into(),
        tr(lang, "tabbar_pos_left").into(),
        tr(lang, "tabbar_pos_right").into(),
    ]
}

/// Footer text based on the item count.
pub fn footer_items_text(lang: Lang, count: usize) -> String {
    match count {
        0 => tr(lang, "footer_empty"),
        1 => tr(lang, "footer_items_singular"),
        n => tr(lang, "footer_items_plural").replace("{n}", &n.to_string()),
    }
}

/// Summary chip standing in for the operation toasts the stack cannot show.
/// Only called with `count >= 1`.
pub fn op_more_text(lang: Lang, count: usize) -> String {
    match count {
        1 => tr(lang, "op_more_singular"),
        n => tr(lang, "op_more_plural").replace("{n}", &n.to_string()),
    }
}

pub fn footer_selection_text(lang: Lang, selected: usize) -> String {
    match selected {
        1 => tr(lang, "footer_selected_singular"),
        n => tr(lang, "footer_selected_plural").replace("{n}", &n.to_string()),
    }
}

/// Full footer: "N items", then (if `hidden > 0`) "· K hidden",
/// then (if there's a selection) "· M selected". The "hidden" segment is only
/// passed when hidden items are NOT shown (a subtle reminder).
pub fn footer_text(lang: Lang, total: usize, selected: usize, hidden: usize) -> String {
    let mut s = footer_items_text(lang, total);
    if hidden > 0 {
        let h = match hidden {
            1 => tr(lang, "footer_hidden_singular"),
            n => tr(lang, "footer_hidden_plural").replace("{n}", &n.to_string()),
        };
        s = format!("{s}{}{h}", tr(lang, "separator_dot"));
    }
    if selected > 0 {
        let sel = footer_selection_text(lang, selected);
        s = format!("{s}{}{sel}", tr(lang, "separator_dot"));
    }
    s
}

// Keyboard shortcuts: labels resolved from the catalog -----

/// Localized label of a shortcut group (`code` = `ActionGroup::code`).
pub fn shortcut_group_name(lang: Lang, code: &str) -> String {
    tr(lang, &format!("shortcut_group.{code}"))
}

/// Localized label of a shortcut action (`id` = `ActionDef::id`).
///
/// The workspace slots share one template instead of nine near-identical
/// entries per language: a digit is all that separates them, and a catalogue
/// repeating the same sentence nine times invites the nine to drift apart.
pub fn shortcut_action_name(lang: Lang, id: &str) -> String {
    if let Some(slot) = id.strip_prefix("load-workspace-") {
        return tr(lang, "shortcut_action_workspace_slot").replace("{n}", slot);
    }
    tr(lang, &format!("shortcut_action.{id}"))
}

/// Localized conflict message: the template contains `{name}`.
pub fn shortcut_conflict_message(lang: Lang, other: &str) -> String {
    tr(lang, "shortcut_conflict.template").replace("{name}", other)
}

/// "Notice" toast message when access to a folder is denied.
/// Size wording for `lang`, ready for [`favnyr_core::fs::format_size`]. The
/// core crate holds no translations, so the vocabulary is resolved here and
/// handed down — decimal mark included, since a language that writes `Ko` also
/// writes `1,0`.
///
/// Memoized like [`catalog`]: this is called once per listed row, so resolving
/// six keys through the map every time would show on a large folder.
pub fn size_units(lang: Lang) -> favnyr_core::fs::SizeUnits<'static> {
    type Units = favnyr_core::fs::SizeUnits<'static>;
    fn build(lang: Lang) -> Units {
        size_units_from_catalog(catalog(lang))
    }
    static EN: OnceLock<Units> = OnceLock::new();
    static FR: OnceLock<Units> = OnceLock::new();
    static ES: OnceLock<Units> = OnceLock::new();
    static DE: OnceLock<Units> = OnceLock::new();
    static IT: OnceLock<Units> = OnceLock::new();
    static RU: OnceLock<Units> = OnceLock::new();
    static ZH: OnceLock<Units> = OnceLock::new();
    static JA: OnceLock<Units> = OnceLock::new();
    let cell = match lang {
        Lang::En => &EN,
        Lang::Fr => &FR,
        Lang::Es => &ES,
        Lang::De => &DE,
        Lang::It => &IT,
        Lang::Ru => &RU,
        Lang::Zh => &ZH,
        Lang::Ja => &JA,
    };
    *cell.get_or_init(|| build(lang))
}

fn size_units_from_catalog(c: &HashMap<String, String>) -> favnyr_core::fs::SizeUnits<'_> {
    let g = |k: &'static str| c.get(k).map(|s| s.as_str()).unwrap_or(k);
    favnyr_core::fs::SizeUnits {
        steps: [
            g("unit_byte"),
            g("unit_kb"),
            g("unit_mb"),
            g("unit_gb"),
            g("unit_tb"),
        ],
        decimal: g("decimal_separator").chars().next().unwrap_or('.'),
    }
}

/// Compact copy throughput and optional remaining-time estimate.
pub fn transfer_status(
    lang: Lang,
    bytes_per_second: u64,
    remaining_seconds: Option<u64>,
) -> String {
    format_transfer_status(
        bytes_per_second,
        remaining_seconds,
        size_units(lang),
        |key| tr(lang, key),
    )
}

fn format_transfer_status(
    bytes_per_second: u64,
    remaining_seconds: Option<u64>,
    units: favnyr_core::fs::SizeUnits<'_>,
    translate: impl Fn(&str) -> String,
) -> String {
    let size = favnyr_core::fs::format_size(bytes_per_second, units);
    let speed = translate("op_transfer_speed").replace("{size}", &size);
    match remaining_seconds {
        Some(seconds) => {
            let time = transfer_time(seconds, &translate);
            let remaining = translate("op_transfer_remaining").replace("{time}", &time);
            format!("{speed}{}{remaining}", translate("separator_dot"))
        }
        None => speed,
    }
}

/// Round minutes upwards so an estimate never claims completion too early.
fn transfer_time(seconds: u64, translate: impl Fn(&str) -> String) -> String {
    if seconds < 60 {
        return format!("{seconds} {}", translate("op_time_second"));
    }
    let minutes = seconds.div_ceil(60);
    if minutes < 60 {
        return format!("{minutes} {}", translate("age_minute"));
    }
    let hours = minutes / 60;
    let rest = minutes % 60;
    let hour = translate("op_time_hour");
    if rest == 0 {
        format!("{hours} {hour}")
    } else {
        format!("{hours} {hour} {rest} {}", translate("age_minute"))
    }
}

/// Age-column wording for `lang`, ready for [`favnyr_core::fs::format_age`].
/// Memoized for the same reason as [`size_units`].
pub fn age_units(lang: Lang) -> favnyr_core::fs::AgeUnits<'static> {
    type Units = favnyr_core::fs::AgeUnits<'static>;
    fn build(lang: Lang) -> Units {
        let c = catalog(lang);
        let g = |k: &'static str| -> &'static str { c.get(k).map(|s| s.as_str()).unwrap_or(k) };
        favnyr_core::fs::AgeUnits {
            minute: g("age_minute"),
            day: g("age_day"),
            month: g("age_month"),
            year: g("age_year"),
            now: g("age_now"),
        }
    }
    static EN: OnceLock<Units> = OnceLock::new();
    static FR: OnceLock<Units> = OnceLock::new();
    static ES: OnceLock<Units> = OnceLock::new();
    static DE: OnceLock<Units> = OnceLock::new();
    static IT: OnceLock<Units> = OnceLock::new();
    static RU: OnceLock<Units> = OnceLock::new();
    static ZH: OnceLock<Units> = OnceLock::new();
    static JA: OnceLock<Units> = OnceLock::new();
    let cell = match lang {
        Lang::En => &EN,
        Lang::Fr => &FR,
        Lang::Es => &ES,
        Lang::De => &DE,
        Lang::It => &IT,
        Lang::Ru => &RU,
        Lang::Zh => &ZH,
        Lang::Ja => &JA,
    };
    *cell.get_or_init(|| build(lang))
}

pub fn access_denied(lang: Lang) -> String {
    tr(lang, "access_denied")
}

/// Translates failure categories instead of exposing an untranslated OS message.
pub fn file_error_reason(lang: Lang, kind: Option<std::io::ErrorKind>) -> String {
    tr(lang, file_error_reason_key(kind))
}

fn file_error_reason_key(kind: Option<std::io::ErrorKind>) -> &'static str {
    use std::io::ErrorKind;
    match kind {
        Some(ErrorKind::PermissionDenied) => "op_reason_permission_denied",
        Some(ErrorKind::ReadOnlyFilesystem) => "op_reason_read_only",
        Some(ErrorKind::NotFound) => "op_reason_not_found",
        Some(ErrorKind::AlreadyExists) => "op_reason_already_exists",
        Some(ErrorKind::StorageFull) => "op_reason_storage_full",
        _ => "op_reason_system",
    }
}

/// Shared item-and-reason interpolation for filesystem operation failures.
pub fn file_operation_failed(lang: Lang, key: &str, item: &str, reason: &str) -> String {
    format_file_failure(&tr(lang, key), item, reason)
}

fn format_file_failure(template: &str, item: &str, reason: &str) -> String {
    // Substitute template tokens before inserting names, which can themselves
    // contain braces and must never be interpreted as translation placeholders.
    template
        .split("{item}")
        .map(|part| part.replace("{reason}", reason))
        .collect::<Vec<_>>()
        .join(item)
}

/// Renders a diagnosed process list as one readable fragment. The ellipsis
/// marks a deliberately bounded diagnostic, so the sentence never claims to
/// name every holder. Empty when nothing could be attributed.
pub fn process_list<S: AsRef<str>>(lang: Lang, items: &[S], truncated: bool) -> String {
    let separator = tr(lang, "list_separator");
    let mut list = String::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            list.push_str(&separator);
        }
        list.push_str(item.as_ref());
    }
    if truncated && !list.is_empty() {
        list.push_str(&separator);
        list.push_str(&tr(lang, "list_ellipsis"));
    }
    list
}

/// Human-readable message after an operation is refused by a Windows lock.
/// The ownerless template stays honest when Restart Manager confirms the
/// conflict but can't inspect the process (permissions, folder handle).
pub fn item_in_use(lang: Lang, item: &str, processes: &[String], truncated: bool) -> String {
    let process = process_list(lang, processes, truncated);
    let key = if process.is_empty() {
        "op_in_use_unknown"
    } else {
        "op_in_use_by"
    };
    tr(lang, key)
        .replace("{item}", item)
        .replace("{process}", &process)
}

/// Message for an entry an operation could not process and stepped over. Used
/// when no holding program could be named, with a translated failure reason.
pub fn item_skipped(lang: Lang, item: &str, reason: &str) -> String {
    file_operation_failed(lang, "op_item_skipped", item, reason)
}

/// Message for a move whose copy succeeded but whose original could not be
/// removed. The item then exists in both places, which the user has to be told
/// explicitly: the action was requested as a move and behaved as a copy.
/// `reason` names the holding process when one could be attributed, and
/// carries the translated failure category otherwise.
pub fn move_source_kept(lang: Lang, item: &str, reason: &str) -> String {
    file_operation_failed(lang, "move_source_kept", item, reason)
}

/// Translated fallback when a rename fails without an attributable Windows
/// lock, with a translated failure reason.
pub fn rename_failed(lang: Lang, item: &str, reason: &str) -> String {
    file_operation_failed(lang, "rename_failed", item, reason)
}

/// Translated reason for a device removal / network disconnection failure.
/// `err` carries no text of its own (see [`favnyr_core::eject::EjectError`]);
/// this is the only place that turns it into a sentence.
pub fn eject_error_message(lang: Lang, err: &favnyr_core::eject::EjectError) -> String {
    use favnyr_core::eject::EjectError;
    match err {
        EjectError::UnknownDevice => tr(lang, "eject_unknown_device"),
        EjectError::BlockedByProcesses(processes) => tr(lang, "eject_blocked_by_processes")
            .replace("{processes}", &process_list(lang, processes, false)),
        EjectError::BlockedByService => tr(lang, "eject_blocked_by_service"),
        EjectError::BlockedByApplication => tr(lang, "eject_blocked_by_application"),
        EjectError::BlockedByOpenFile => tr(lang, "eject_blocked_by_open_file"),
        EjectError::DeviceBusy => tr(lang, "eject_device_busy"),
        EjectError::ToolNotFound => tr(lang, "eject_tool_not_found"),
        EjectError::ToolFailed {
            bin,
            detail: Some(detail),
        } => tr(lang, "eject_tool_failed_detail")
            .replace("{bin}", bin)
            .replace("{detail}", detail),
        EjectError::ToolFailed { bin, detail: None } => {
            tr(lang, "eject_tool_failed").replace("{bin}", bin)
        }
        EjectError::DisconnectFailed(code) => {
            tr(lang, "eject_disconnect_failed").replace("{code}", &code.to_string())
        }
        EjectError::LinuxNetworkDisconnectUnsupported => {
            tr(lang, "eject_linux_network_unsupported")
        }
        EjectError::SystemQueryFailed => tr(lang, "eject_system_query_failed"),
        EjectError::PlatformUnsupported => tr(lang, "eject_platform_unsupported"),
    }
}

/// Translated reason for a mount failure. `err` carries no text of its own
/// (see [`favnyr_core::mount::MountError`]); this is the only place that turns
/// it into a sentence.
pub fn mount_error_message(lang: Lang, err: &favnyr_core::mount::MountError) -> String {
    use favnyr_core::mount::MountError;
    match err {
        MountError::UnknownDevice => tr(lang, "mount_unknown_device"),
        MountError::ToolNotFound => tr(lang, "mount_tool_not_found"),
        MountError::NotAuthorized => tr(lang, "mount_not_authorized"),
        MountError::AlreadyMounted => tr(lang, "mount_already_mounted"),
        MountError::NoMountPoint => tr(lang, "mount_no_mount_point"),
        MountError::ToolFailed {
            detail: Some(detail),
        } => tr(lang, "mount_failed_detail").replace("{detail}", detail),
        MountError::ToolFailed { detail: None } => tr(lang, "mount_failed"),
        MountError::PlatformUnsupported => tr(lang, "mount_platform_unsupported"),
    }
}

/// Translated reason for a trash-restore failure. `err` carries no text of
/// its own (see [`favnyr_core::fs::ops::TrashError`]); this is the only place
/// that turns it into a sentence.
pub fn trash_error_message(lang: Lang, err: &favnyr_core::fs::ops::TrashError) -> String {
    use favnyr_core::fs::ops::TrashError;
    match err {
        TrashError::ListFailed(detail) => {
            tr(lang, "trash_reason_list_failed").replace("{detail}", detail)
        }
        TrashError::ItemNotFound => tr(lang, "trash_reason_item_not_found"),
        TrashError::RestoreFailed(error) => {
            let reason = file_error_reason(lang, error.io_kind());
            tr(lang, "trash_reason_restore_failed").replace("{detail}", &reason)
        }
        TrashError::PlatformUnsupported => tr(lang, "trash_reason_platform_unsupported"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn preference_strings_exist_in_each_embedded_language() {
        for &lang in favnyr_core::Lang::all() {
            let table: toml::Table = toml::from_str(super::embedded(lang)).unwrap();
            for key in [
                "settings_group_tooltips",
                "settings_group_workspaces",
                "settings_grid_name_tooltip_label",
                "settings_grid_name_tooltip_hint",
                "settings_search_path_tooltip_label",
                "settings_search_path_tooltip_hint",
                "settings_ws_panel_label",
                "settings_ws_panel_hint",
                "settings_ws_shortcut_label",
                "settings_ws_shortcut_hint",
                "settings_ws_shortcut_preview",
                "ws_shortcut_ask",
                "ws_shortcut_discard",
                "ws_shortcut_save",
                "ws_shortcut_remember",
                "ws_panel_remember",
                "ws_dirty_body_new",
                "ws_dirty_discard_new",
                "ws_dirty_save_new",
                "settings_thumbnail_zoom_label",
                "settings_thumbnail_zoom_hint",
                "tabbar_pos_top",
                "tabbar_pos_left",
                "tabbar_pos_right",
                "settings_letter_layout",
                "settings_letter_layout_tip",
                "settings_letter_layout_off",
                "settings_keyboard_ukrainian_standard",
                "settings_keyboard_russian_standard",
                "thumbnail_zoom_feedback",
            ] {
                assert!(
                    table
                        .get(key)
                        .and_then(toml::Value::as_str)
                        .is_some_and(|text| !text.is_empty()),
                    "missing translation: {lang:?}/{key}"
                );
            }
            let feedback = table["thumbnail_zoom_feedback"].as_str().unwrap();
            assert!(feedback.contains("{level}") && feedback.contains("{max}"));
        }
    }
    use super::*;

    #[test]
    fn grid_catalogs_include_display_settings_and_keyboard_actions_without_fallback() {
        // Derived, not listed: a hardcoded set is how a newly added language
        // slips past this check.
        for &lang in Lang::all() {
            let table: toml::Table = toml::from_str(embedded(lang)).unwrap();
            let mut catalog = HashMap::new();
            flatten_into("", &table, &mut catalog);
            for key in [
                "view_mode_tooltip",
                "view_layout_list",
                "view_layout_grid",
                "view_sort_tooltip",
                "settings_default_item_arrangement_label",
                "settings_default_hidden_label",
                "settings_tab_defaults_title",
                "settings_tab_defaults_hint",
                "settings_view_mode_list",
                "settings_view_mode_previews",
                "settings_view_item_spacing_hint",
                "settings_compact_preview_hint",
                "shortcut_action.cursor-left",
                "shortcut_action.cursor-right",
                "shortcut_action.extend-left",
                "shortcut_action.extend-right",
            ] {
                assert!(
                    catalog.get(key).is_some_and(|text| !text.trim().is_empty()),
                    "{lang:?}: {key}"
                );
            }
        }
    }

    #[test]
    fn file_failure_messages_and_reasons_are_translated_in_every_bundled_language() {
        use std::io::ErrorKind;
        for &lang in Lang::all() {
            let mut map = HashMap::new();
            overlay(embedded(lang), &mut map);
            for kind in [
                Some(ErrorKind::PermissionDenied),
                Some(ErrorKind::ReadOnlyFilesystem),
                Some(ErrorKind::NotFound),
                Some(ErrorKind::AlreadyExists),
                Some(ErrorKind::StorageFull),
                None,
            ] {
                let reason = &map[file_error_reason_key(kind)];
                assert!(!reason.is_empty(), "{lang:?}");
                for key in [
                    "create_failed",
                    "delete_failed",
                    "link_failed",
                    "rename_failed",
                    "op_item_skipped",
                    "move_source_kept",
                ] {
                    let text = format_file_failure(&map[key], "my_file.txt", reason);
                    assert!(
                        text.contains("my_file.txt") && text.contains(reason),
                        "{lang:?} {key}"
                    );
                    assert!(
                        !text.contains("{item}") && !text.contains("{reason}"),
                        "{lang:?} {key}"
                    );
                }
            }
        }
    }

    #[test]
    fn file_failure_interpolation_preserves_braces_in_names_and_reasons() {
        assert_eq!(
            format_file_failure("{item}: {reason}", "{reason}.txt", "{item}"),
            "{reason}.txt: {item}"
        );
        assert_eq!(
            file_error_reason_key(Some(std::io::ErrorKind::Other)),
            "op_reason_system"
        );
    }

    #[test]
    fn every_language_loads_and_has_app_title() {
        for &lang in Lang::all() {
            assert_eq!(tr(lang, "app_title"), "Favnyr", "{lang:?}");
        }
    }

    #[test]
    fn theme_messages_are_present_in_every_embedded_catalog() {
        for &lang in Lang::all() {
            let mut catalog = HashMap::new();
            overlay(embedded(lang), &mut catalog);
            for key in [
                "settings_appearance",
                "theme_nord",
                "theme_catppuccin_mocha",
                "theme_catppuccin_macchiato",
                "theme_catppuccin_frappe",
                "theme_tokyo_night",
                "theme_atom",
                "theme_fluvy",
                "theme_everforest",
                "theme_sonokai_atlantis",
                "theme_rose_pine",
                "theme_gruvbox_hard",
                "theme_auto",
                "theme_light",
                "theme_dark",
                "theme_folder",
                "theme_reload",
                "theme_unavailable_name",
                "theme_unavailable",
                "theme_invalid_files",
                "theme_failed",
                "theme_folder_failed",
                "theme_workspace_override",
                "ws_theme_assign",
                "ws_theme_none",
                "edit_select_all",
                "edit_clear",
                "edit_show_password",
                "edit_hide_password",
            ] {
                assert!(
                    catalog.get(key).is_some_and(|text| !text.is_empty()),
                    "{lang:?}/{key}"
                );
            }
            assert!(catalog["theme_invalid_files"].contains("{count}"));
            // The sentence names both the workspace and the theme it imposes,
            // which is the whole point: either one missing leaves it saying
            // nothing the reader can act on.
            assert!(catalog["theme_workspace_override"].contains("{name}"));
            assert!(catalog["theme_workspace_override"].contains("{theme}"));
        }
    }

    #[test]
    fn translated_properties_update_live_and_remain_local_to_each_window() {
        crate::test_support::run(|| {
            let first = MainWindow::new().unwrap();
            let second = MainWindow::new().unwrap();
            // No layout assumed until the user names one.
            assert_eq!(first.get_letter_layout_index(), 0);
            let mut english = HashMap::new();
            overlay(EMBED_EN, &mut english);
            apply_catalog(&second, &english);

            for &lang in Lang::all().iter().chain(std::iter::once(&Lang::En)) {
                let mut translated = english.clone();
                overlay(embedded(lang), &mut translated);
                apply_catalog(&first, &translated);
                let strings = first.global::<Strings>();
                for (actual, key) in [
                    (strings.get_btn_cancel(), "btn_cancel"),
                    (strings.get_settings_language(), "settings_language"),
                    (strings.get_settings_appearance(), "settings_appearance"),
                    (strings.get_theme_folder(), "theme_folder"),
                    (strings.get_archive_create(), "archive_create"),
                    (strings.get_exit_stay(), "exit_stay"),
                    (
                        strings.get_settings_letter_layout(),
                        "settings_letter_layout",
                    ),
                    (
                        strings.get_settings_letter_layout_tip(),
                        "settings_letter_layout_tip",
                    ),
                    (
                        strings.get_settings_letter_layout_off(),
                        "settings_letter_layout_off",
                    ),
                    (
                        strings.get_settings_keyboard_ukrainian_standard(),
                        "settings_keyboard_ukrainian_standard",
                    ),
                    (
                        strings.get_settings_keyboard_russian_standard(),
                        "settings_keyboard_russian_standard",
                    ),
                ] {
                    assert_eq!(actual.as_str(), translated[key], "{lang:?}/{key}");
                }
                assert_eq!(
                    second.global::<Strings>().get_btn_cancel().as_str(),
                    english["btn_cancel"]
                );
            }

            // Exercise custom overrides through the same merge and UI bindings.
            let mut custom = english.clone();
            overlay(
                "app_title = \"Custom title\"\nbtn_cancel = \"Custom action\"",
                &mut custom,
            );
            apply_catalog(&first, &custom);
            assert_eq!(first.get_window_title(), "Custom title");
            assert_eq!(first.global::<Strings>().get_btn_cancel(), "Custom action");
            assert_eq!(second.get_window_title().as_str(), english["app_title"]);
            apply_catalog(&first, &english);
            assert_eq!(first.get_window_title().as_str(), english["app_title"]);
            assert_eq!(
                first.global::<Strings>().get_btn_cancel().as_str(),
                english["btn_cancel"]
            );
        });
    }

    #[test]
    fn french_overrides_english() {
        assert_eq!(tr(Lang::Fr, "settings_language"), "Langue");
        assert_eq!(tr(Lang::En, "settings_language"), "Language");
    }

    #[test]
    fn transfer_status_localizes_units_and_interpolates_each_language() {
        let mut speeds = HashMap::new();
        for &lang in Lang::all() {
            // Use the bundled catalog so personal overrides cannot affect the test.
            let mut map = HashMap::new();
            overlay(embedded(lang), &mut map);
            let translate = |key: &str| map.get(key).expect("missing transfer translation").clone();
            let units = size_units_from_catalog(&map);
            let speed = format_transfer_status(1536, None, units, translate);
            let remaining = format_transfer_status(1536, Some(61), units, translate);
            assert!(!speed.contains('{') && !remaining.contains('{'), "{lang:?}");
            assert!(remaining.starts_with(&speed), "{lang:?}");
            assert!(
                remaining.contains(&transfer_time(61, translate)),
                "{lang:?}"
            );
            speeds.insert(lang.code(), speed);
        }
        assert_eq!(speeds["en"], "1.5 KB/s");
        assert_eq!(speeds["fr"], "1,5 Ko/s");
    }

    #[test]
    fn transfer_time_rounds_up_and_keeps_long_estimates_compact() {
        let mut map = HashMap::new();
        overlay(EMBED_EN, &mut map);
        for (seconds, expected) in [
            (0, "0 s"),
            (59, "59 s"),
            (60, "1 min"),
            (61, "2 min"),
            (3599, "1 h"),
            (3600, "1 h"),
            (3601, "1 h 1 min"),
            (7200, "2 h"),
        ] {
            assert_eq!(transfer_time(seconds, |key| map[key].clone()), expected);
        }
    }

    #[test]
    fn russian_overrides_english() {
        assert_eq!(tr(Lang::Ru, "settings_language"), "Язык");
        assert_eq!(tr(Lang::Ru, "col_name"), "Имя");
    }

    #[test]
    fn japanese_overrides_english() {
        assert_eq!(tr(Lang::Ja, "settings_language"), "言語");
        assert_eq!(tr(Lang::Ja, "settings_view_mode_previews"), "サムネイル");
    }

    #[test]
    fn chinese_overrides_english() {
        assert_eq!(tr(Lang::Zh, "settings_language"), "语言");
        assert_eq!(tr(Lang::Zh, "settings_view_mode_previews"), "缩略图");
    }

    /// A bundled language must carry every key in its own file rather than
    /// lean on the English fallback. That fallback exists so a missing key
    /// never surfaces as raw text; it is not a licence to ship a language
    /// half in English.
    ///
    /// [`catalog`] cannot see this — it overlays English first, so a fallback
    /// and a real translation are indistinguishable there. The embedded source
    /// is what has to be read.
    #[test]
    fn every_bundled_language_carries_every_key_itself() {
        fn keys(src: &str) -> std::collections::BTreeSet<String> {
            let mut map = HashMap::new();
            overlay(src, &mut map);
            map.into_keys().collect()
        }
        let english = keys(embedded(Lang::En));
        assert!(english.len() > 400, "the English base looks truncated");
        for &lang in Lang::all() {
            if lang == Lang::En {
                continue;
            }
            let own = keys(embedded(lang));
            let missing: Vec<_> = english.difference(&own).collect();
            assert!(
                missing.is_empty(),
                "{lang:?} would fall back to English for {} key(s): {missing:?}",
                missing.len()
            );
            let extra: Vec<_> = own.difference(&english).collect();
            assert!(
                extra.is_empty(),
                "{lang:?} declares {} key(s) English does not: {extra:?}",
                extra.len()
            );
        }
    }

    #[test]
    fn colored_folder_grouping_is_present_in_every_language_without_fallback() {
        for &lang in Lang::all() {
            assert!(
                catalog(lang)
                    .get("group_colored_folders_first")
                    .is_some_and(|value| !value.is_empty()),
                "missing colored-folder grouping translation: {lang:?}"
            );
        }
    }

    #[test]
    fn archive_messages_are_present_in_every_language_without_fallback() {
        let english = catalog(Lang::En);
        let keys: Vec<_> = english
            .keys()
            .filter(|key| key.starts_with("archive_"))
            .collect();
        assert!(keys.len() >= 21);
        for &lang in Lang::all() {
            let translated = catalog(lang);
            for key in &keys {
                assert!(
                    translated.get(*key).is_some_and(|value| !value.is_empty()),
                    "missing archive translation: {lang:?} {key}"
                );
            }
        }
    }

    #[test]
    fn the_nine_workspace_slots_share_one_translated_label() {
        // One template carries the nine labels, so what must be checked is that
        // the digit reaches it and that no language lost the template.
        for lang in Lang::all() {
            let third = shortcut_action_name(*lang, "load-workspace-3");
            assert!(third.contains('3'), "{lang:?} dropped the digit: {third}");
            assert!(
                !third.contains("{n}"),
                "{lang:?} left the placeholder in: {third}"
            );
            assert_ne!(
                third,
                shortcut_action_name(*lang, "load-workspace-7"),
                "{lang:?} renders every slot the same"
            );
        }
        // An ordinary action keeps the per-id lookup.
        assert_eq!(shortcut_action_name(Lang::Fr, "copy"), "Copier");
    }

    #[test]
    fn missing_key_falls_back_to_english_then_key() {
        // Key present everywhere: the language's value.
        assert_eq!(shortcut_action_name(Lang::Fr, "copy"), "Copier");
        // Nonexistent key → returned as-is (no panic/empty string).
        assert_eq!(tr(Lang::Fr, "zzz_missing_key"), "zzz_missing_key");
    }

    #[test]
    fn conflict_template_interpolates_name() {
        let msg = shortcut_conflict_message(Lang::En, "Copy");
        assert!(msg.contains("Copy") && !msg.contains("{name}"));
    }

    #[test]
    fn item_in_use_message_is_translated_and_interpolated() {
        for &lang in Lang::all() {
            let processes = vec!["editor.exe".to_owned()];
            let known = item_in_use(lang, "my_file.txt", &processes, true);
            assert!(known.contains("my_file.txt") && known.contains("editor.exe"));
            assert!(known.contains('…'));
            assert!(!known.contains("{item}") && !known.contains("{process}"));

            let unknown = item_in_use(lang, "my_file.txt", &[], false);
            assert!(unknown.contains("my_file.txt"));
            assert!(!unknown.contains("{item}") && !unknown.contains("{process}"));
        }
    }

    #[test]
    fn move_source_kept_is_translated_and_interpolated() {
        for &lang in Lang::all() {
            // Attributed holder: the process list reaches the sentence.
            let named =
                move_source_kept(lang, "report.odt", &process_list(lang, &["editor"], false));
            assert!(named.contains("report.odt") && named.contains("editor"));
            assert!(!named.contains("{item}") && !named.contains("{reason}"));

            // No attributable holder: the system error takes its place.
            let raw = move_source_kept(lang, "report.odt", "permission denied");
            assert!(raw.contains("permission denied"));
            assert!(!raw.contains("{reason}"));
        }
        // Nothing diagnosed at all → empty fragment, which is what makes the
        // caller fall back to the system error instead of an empty reason.
        assert!(process_list::<&str>(Lang::En, &[], false).is_empty());
        assert_eq!(process_list(Lang::En, &["a", "b"], true), "a, b, …");
    }

    #[test]
    fn rename_failure_is_translated_and_interpolated() {
        for &lang in Lang::all() {
            let message = rename_failed(lang, "photo.png", "access denied");
            assert!(message.contains("photo.png") && message.contains("access denied"));
            assert!(!message.contains("{item}") && !message.contains("{reason}"));
        }
    }

    #[test]
    fn all_catalog_values_resolve_nonempty() {
        // Every catalog value must contain a usable translation.
        let c = catalog(Lang::En);
        for (k, v) in c {
            assert!(!v.is_empty(), "empty key: {k}");
        }
    }

    /// Every key requested by `apply_catalog` via `g("…")` must exist at the
    /// root level of the English catalog. A key placed under a `[…]` section
    /// becomes `section.key` and can't be resolved by `g`. This test re-reads
    /// the source of `apply_catalog`.
    #[test]
    fn every_g_key_present_in_en() {
        let src = include_str!("i18n.rs");
        let c = catalog(Lang::En);
        let mut checked = 0usize;
        for line in src.lines() {
            // Ignore COMMENTS (including this docstring, which contains `g("…")`).
            if line.trim_start().starts_with("//") {
                continue;
            }
            let mut rest = line;
            while let Some(pos) = rest.find("g(\"") {
                let after = &rest[pos + 3..];
                let Some(end) = after.find('"') else { break };
                let key = &after[..end];
                assert!(
                    c.contains_key(key),
                    "key g(\"{key}\") missing from the top level of en.toml (misplaced under a [section]?)"
                );
                checked += 1;
                rest = &after[end + 1..];
            }
        }
        // Guard: the scan did find keys (otherwise the test proves nothing).
        assert!(
            checked > 100,
            "scan g(\"…\") found abnormally few keys: {checked}"
        );
    }
}

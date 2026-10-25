//! Stable semantic color roles shared by built-in and personal palettes.

use serde::Deserialize;

macro_rules! color_roles {
    ($( $role:ident ),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
        #[serde(rename_all = "snake_case")]
        #[repr(usize)]
        pub enum ColorRole { $( $role ),+ }

        impl ColorRole {
            pub const ALL: &'static [Self] = &[$( Self::$role ),+];
        }
    };
}

color_roles! {
    Background,
    Surface,
    Toolbar,
    AlternateBackground,
    Foreground,
    Border,
    AccentBackground,
    AccentForeground,
    Danger,
    DangerForeground,
    Success,
    Warning,
    Extension,
    Track,
    DiskNormal,
    DiskWarning,
    DiskCritical,
    DiskNormalForeground,
    DiskWarningForeground,
    DiskCriticalForeground,
    ToastSuccess,
    ToastInfo,
    ToastDanger,
    TooltipBackground,
    TooltipForeground,
    Shadow,
    ModalOverlay,
    FileApplication,
    FileArchive,
    FileAudio,
    FileDocument,
    FileImage,
    FileVideo,
    FileRegistry,
    FolderDefault,
    FolderRed,
    FolderGreen,
    FolderTeal,
    FolderBlue,
    FolderViolet,
    FolderPink,
    FolderSlate,
    AgeHour,
    AgeHours,
    AgeDay,
    AgeWeek,
    AgeMonth,
    AgeYear,
    AgeOlder,
    FolderRedForeground,
    FolderGreenForeground,
    FolderTealForeground,
    FolderBlueForeground,
    FolderVioletForeground,
    FolderPinkForeground,
    FolderSlateForeground,
}

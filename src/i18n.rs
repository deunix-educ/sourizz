//! English / French UI strings (tray menu and settings window).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// Resolve the `general.language` setting (`auto`, `en`, `fr`) against the environment.
    pub fn resolve(setting: &str) -> Lang {
        match setting {
            "fr" => Lang::Fr,
            "en" => Lang::En,
            _ => {
                let env = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
                    .iter()
                    .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
                    .unwrap_or_default();
                if env.to_ascii_lowercase().starts_with("fr") { Lang::Fr } else { Lang::En }
            }
        }
    }
}

const TABLE: &[(&str, &str, &str)] = &[
    // tray + general
    ("enabled", "Enabled", "Activé"),
    ("spotlight", "Spotlight", "Projecteur"),
    ("ring", "Ring cursor", "Anneau de curseur"),
    ("trail", "Cursor trail", "Traînée du curseur"),
    ("clicks", "Click effects", "Effets de clic"),
    ("sound", "Click sound", "Son de clic"),
    ("shake", "Shake to locate", "Secouer pour localiser"),
    ("annotate", "Hold-key drawing", "Dessin en maintenant une touche"),
    ("keystrokes", "Show keystrokes", "Afficher les frappes"),
    ("focus", "Focus mode", "Mode focus"),
    ("annotation_mode", "Annotation mode", "Mode annotation"),
    ("clear_annotations", "Clear annotations", "Effacer les annotations"),
    ("show_click", "Show click effect", "Afficher l'effet de clic"),
    ("settings", "Settings…", "Réglages…"),
    ("quit", "Quit", "Quitter"),
    ("effects", "Effects", "Effets"),
    ("styles", "Styles", "Styles"),
    ("spot_shape", "Spotlight shape", "Forme du projecteur"),
    ("ring_style", "Ring style", "Style de l'anneau"),
    ("trail_style", "Trail style", "Style de la traînée"),
    ("click_style", "Click effect", "Effet de clic"),
    ("key_style", "Keystroke style", "Style des frappes"),
    // option values
    ("opt.circle", "Circle", "Cercle"),
    ("opt.hbar", "Horizontal bar", "Barre horizontale"),
    ("opt.vbar", "Vertical bar", "Barre verticale"),
    ("opt.classic", "Classic", "Classique"),
    ("opt.neon", "Neon", "Néon"),
    ("opt.comet", "Comet", "Comète"),
    ("opt.dots", "Dots", "Points"),
    ("opt.ribbon", "Ribbon", "Ruban"),
    ("opt.rainbow", "Rainbow", "Arc-en-ciel"),
    ("opt.stars", "Stars", "Étoiles"),
    ("opt.kawaii", "Kawaii", "Kawaii"),
    ("opt.sparkles", "Sparkles", "Paillettes"),
    ("opt.single_ring", "Single ring", "Anneau simple"),
    ("opt.ripple_wave", "Ripple wave", "Ondulation"),
    ("opt.burst", "Burst", "Éclat"),
    ("opt.pulse", "Pulse", "Pulsation"),
    ("opt.glass", "Glass", "Verre"),
    ("opt.keycaps", "Keycaps", "Touches"),
    ("opt.gradient", "Gradient", "Dégradé"),
    ("opt.minimal", "Minimal", "Minimal"),
    ("opt.auto", "Automatic", "Automatique"),
    ("opt.x11", "X11", "X11"),
    ("opt.wayland", "Wayland", "Wayland"),
    ("opt.bottom_center", "Bottom centre", "Bas centre"),
    ("opt.bottom_left", "Bottom left", "Bas gauche"),
    ("opt.bottom_right", "Bottom right", "Bas droite"),
    ("opt.top_center", "Top centre", "Haut centre"),
    // settings window
    ("app.title", "Sourizz settings", "Réglages de Sourizz"),
    ("tab.effects", "Effects", "Effets"),
    ("tab.clicks", "Clicks & sound", "Clics et son"),
    ("tab.draw", "Drawing", "Dessin"),
    ("tab.keys", "Keystrokes & focus", "Frappes et focus"),
    ("tab.hotkeys", "Hotkeys", "Raccourcis"),
    ("tab.general", "General", "Général"),
    ("f.size", "Size", "Taille"),
    ("f.radius", "Radius", "Rayon"),
    ("f.thickness", "Thickness", "Épaisseur"),
    ("f.color", "Colour", "Couleur"),
    ("f.opacity", "Opacity", "Opacité"),
    ("f.softness", "Edge softness", "Douceur du bord"),
    ("f.fill", "Fill", "Remplissage"),
    ("f.pulse", "Pulse speed", "Vitesse de pulsation"),
    ("f.shape", "Shape", "Forme"),
    ("f.style", "Style", "Style"),
    ("f.lifetime", "Lifetime (ms)", "Durée (ms)"),
    ("f.duration", "Duration (ms)", "Durée (ms)"),
    ("f.show_on_clicks", "Show on mouse clicks", "Afficher aux clics de souris"),
    ("f.tap_key", "Tap key triggers effect", "Touche qui déclenche l'effet (appui bref)"),
    ("f.left", "Left button", "Bouton gauche"),
    ("f.middle", "Middle button", "Bouton du milieu"),
    ("f.right", "Right button", "Bouton droit"),
    ("f.volume", "Volume", "Volume"),
    ("f.sound_file", "Custom sound file (WAV)", "Fichier son personnalisé (WAV)"),
    ("f.on_release", "Also play on release", "Jouer aussi au relâchement"),
    ("f.sensitivity", "Sensitivity", "Sensibilité"),
    ("f.scale", "Enlarge by", "Agrandir de"),
    ("f.hold_key", "Hold key to draw", "Touche à maintenir pour dessiner"),
    ("f.width", "Pen width", "Épaisseur du stylo"),
    ("f.fade", "Fade strokes after release", "Faire disparaître les traits"),
    ("f.fade_ms", "Fade time (ms)", "Temps de disparition (ms)"),
    ("f.position", "Position", "Position"),
    ("f.only_shortcuts", "Only show shortcuts (hide plain typing)", "Seulement les raccourcis (masquer la saisie)"),
    ("f.accent", "Accent colour", "Couleur d'accent"),
    ("f.language", "Language", "Langue"),
    ("f.backend", "Display backend", "Serveur d'affichage"),
    ("f.fps", "Max frame rate", "Images/s maximum"),
    (
        "hotkeys.help",
        "Format: Ctrl+Alt+Shift+S. Leave empty to disable. On Wayland, bind `sourizz toggle …` in your desktop's shortcut settings instead.",
        "Format : Ctrl+Alt+Shift+S. Laisser vide pour désactiver. Sous Wayland, associez plutôt `sourizz toggle …` dans les raccourcis de votre bureau.",
    ),
    ("hk.toggle_all", "Toggle everything", "Tout activer / désactiver"),
    ("hk.annotate_mode", "Annotation mode", "Mode annotation"),
    ("hk.clear_annotations", "Clear annotations", "Effacer les annotations"),
    ("hk.show_click", "Show click effect", "Afficher l'effet de clic"),
    (
        "saved",
        "Changes apply instantly and are saved automatically.",
        "Les changements s'appliquent immédiatement et sont enregistrés automatiquement.",
    ),
    (
        "daemon_down",
        "The Sourizz daemon is not running — start it with `sourizz`.",
        "Le démon Sourizz n'est pas lancé — démarrez-le avec `sourizz`.",
    ),
    ("reset", "Reset to defaults", "Rétablir les valeurs par défaut"),
];

/// Look up a UI string; unknown keys come back as themselves so gaps are visible, not fatal.
pub fn t(lang: Lang, key: &'static str) -> &'static str {
    TABLE.iter().find(|(k, _, _)| *k == key).map(|(_, en, fr)| if lang == Lang::Fr { *fr } else { *en }).unwrap_or(key)
}

/// Label for an enum value id such as `stars` or `ripple_wave`.
pub fn opt(lang: Lang, id: &str) -> String {
    TABLE
        .iter()
        .find(|(k, _, _)| k.strip_prefix("opt.") == Some(id))
        .map(|(_, en, fr)| (if lang == Lang::Fr { *fr } else { *en }).to_string())
        .unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    #[test]
    fn keys_unique_and_filled() {
        let mut seen = std::collections::HashSet::new();
        for (k, en, fr) in TABLE {
            assert!(seen.insert(*k), "duplicate key {k}");
            assert!(!en.is_empty() && !fr.is_empty(), "empty text for {k}");
        }
    }

    #[test]
    fn every_enum_value_is_translated() {
        let ids: Vec<&str> = BackendKind::ALL
            .iter()
            .map(|v| v.id())
            .chain(SpotShape::ALL.iter().map(|v| v.id()))
            .chain(RingStyle::ALL.iter().map(|v| v.id()))
            .chain(TrailStyle::ALL.iter().map(|v| v.id()))
            .chain(ClickStyle::ALL.iter().map(|v| v.id()))
            .chain(KeyStyle::ALL.iter().map(|v| v.id()))
            .chain(KeyPosition::ALL.iter().map(|v| v.id()))
            .collect();
        for id in ids {
            assert_ne!(opt(Lang::En, id), id, "missing translation for option {id}");
            assert_ne!(opt(Lang::Fr, id), id, "missing French translation for option {id}");
        }
    }

    #[test]
    fn french_differs() {
        assert_eq!(t(Lang::En, "quit"), "Quit");
        assert_eq!(t(Lang::Fr, "quit"), "Quitter");
        assert_eq!(t(Lang::En, "no such key"), "no such key");
    }
}

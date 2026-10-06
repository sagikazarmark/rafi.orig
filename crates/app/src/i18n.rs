//! Hungarian and English strings for the public pages.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Hu,
}

impl Lang {
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("hu") {
            Self::Hu
        } else {
            Self::En
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Hu => "hu",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::En => Self::Hu,
            Self::Hu => Self::En,
        }
    }

    /// Looks up a string by key; unknown keys return the key itself.
    pub fn t(self, key: &str) -> &'static str {
        let (en, hu) = match key {
            "join_title" => ("Join the raffle", "Csatlakozz a sorsoláshoz"),
            "display_name" => ("Name shown on the screen", "A kivetítőn megjelenő név"),
            "display_name_hint" => (
                "A first name or nickname is enough.",
                "Elég a keresztneved vagy egy becenév.",
            ),
            "email" => ("Email", "E-mail"),
            "email_hint" => (
                "Only to tell entries apart and to deliver prizes. Never shown.",
                "Csak a jelentkezések megkülönböztetésére és a nyeremény átadására használjuk. Sehol nem jelenik meg.",
            ),
            "accept_rules" => (
                "I accept the raffle rules and the privacy notice.",
                "Elfogadom a sorsolás szabályait és az adatkezelési tájékoztatót.",
            ),
            "marketing" => (
                "Email me about future meetups (optional).",
                "Kérek e-mailt a következő meetupokról (nem kötelező).",
            ),
            "join" => ("Join", "Csatlakozom"),
            "rules" => ("Rules", "Szabályok"),
            "privacy" => ("Privacy notice", "Adatkezelési tájékoztató"),
            "youre_in" => ("You're in!", "Bent vagy!"),
            "claim_code" => ("Your claim code", "Az átvételi kódod"),
            "claim_code_hint" => (
                "If you win, show this code to the organizer.",
                "Ha nyersz, ezt a kódot mutasd meg a szervezőnek.",
            ),
            "keep_open" => (
                "You can close this page. Come back any time — your entry is safe.",
                "Bezárhatod ezt az oldalt, bármikor visszatérhetsz — a jelentkezésed megmarad.",
            ),
            "keep_screen_on" => ("Keep my screen on", "Maradjon bekapcsolva a képernyő"),
            "screen_on" => ("Screen will stay on", "A képernyő bekapcsolva marad"),
            "waiting" => (
                "Waiting for the draw to start…",
                "Várakozás a sorsolás kezdetére…",
            ),
            "draw_starting" => ("The draw is starting!", "Kezdődik a sorsolás!"),
            "drawing" => ("Drawing", "Sorsolás"),
            "still_in" => ("You're still in the game.", "Még játékban vagy."),
            "you_won" => ("YOU WON!", "NYERTÉL!"),
            "go_to_organizer" => (
                "Go to the organizer and show your claim code.",
                "Menj a szervezőhöz és mutasd meg az átvételi kódod.",
            ),
            "not_this_time" => ("Not this time", "Most nem nyertél"),
            "prizes_left" => ("prizes left", "nyeremény van még"),
            "thanks" => ("Thanks for playing!", "Köszönjük, hogy játszottál!"),
            "thanks_won" => (
                "Congratulations again — enjoy your prize!",
                "Még egyszer gratulálunk, jó szórakozást a nyereményhez!",
            ),
            "reg_closed" => ("Registration is closed.", "A jelentkezés lezárult."),
            "reg_not_open" => (
                "Registration is not open yet. Keep an eye on the screen!",
                "A jelentkezés még nem indult el. Figyeld a kivetítőt!",
            ),
            "already_in" => (
                "This email is already entered. Your entry is safe — watch the screen. If you win, the organizer will confirm you by email.",
                "Ezzel az e-mail címmel már van jelentkezés. A jelentkezésed megmarad — figyeld a kivetítőt. Ha nyersz, a szervező az e-mail címed alapján azonosít.",
            ),
            "err_name" => (
                "Please enter a name (1–24 characters).",
                "Adj meg egy nevet (1–24 karakter).",
            ),
            "err_email" => (
                "Please enter a valid email address.",
                "Adj meg egy érvényes e-mail címet.",
            ),
            "err_rules" => (
                "You need to accept the rules to take part.",
                "A részvételhez el kell fogadnod a szabályokat.",
            ),
            "err_bot" => (
                "Could not verify that you are human. Please reload the page and try again.",
                "Nem sikerült ellenőrizni, hogy nem robot vagy. Töltsd újra az oldalt és próbáld újra.",
            ),
            "err_rate" => (
                "Too many attempts. Please wait a minute.",
                "Túl sok próbálkozás. Várj egy percet.",
            ),
            "err_name_blocked" => (
                "Please choose a different name.",
                "Kérlek válassz másik nevet.",
            ),
            "no_phone" => (
                "No smartphone? Come to the organizer!",
                "Nincs okostelefonod? Gyere a szervezőhöz!",
            ),
            "scan_to_join" => ("Scan to join", "Szkenneld be a csatlakozáshoz"),
            "or_visit" => ("or visit", "vagy nyisd meg:"),
            "code" => ("code", "kód"),
            "entrants" => ("entrants", "résztvevő"),
            "prizes" => ("Prizes", "Nyeremények"),
            "locking" => (
                "Locking in public randomness",
                "Nyilvános véletlenszám rögzítése",
            ),
            "drand_round" => ("drand round", "drand kör"),
            "winners" => ("Winners", "Nyertesek"),
            "verify_link" => ("Check the draw yourself", "Ellenőrizd a sorsolást"),
            "connecting" => ("Connecting…", "Kapcsolódás…"),
            "not_found" => ("This raffle does not exist.", "Ez a sorsolás nem létezik."),
            "test_mode" => ("TEST MODE", "TESZT MÓD"),
            "registration_soon" => ("Registration opens soon", "Hamarosan indul a jelentkezés"),
            "not_here" => ("not here", "nem volt jelen"),
            "declined" => ("declined", "visszautasította"),
            _ => return leak(key),
        };
        match self {
            Self::En => en,
            Self::Hu => hu,
        }
    }
}

fn leak(key: &str) -> &'static str {
    // Unknown keys are a programming error; make them visible but don't panic.
    Box::leak(key.to_owned().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::Lang;

    #[test]
    fn known_keys_translate() {
        assert_eq!(Lang::En.t("join"), "Join");
        assert_eq!(Lang::Hu.t("join"), "Csatlakozom");
        assert_eq!(Lang::parse("HU"), Lang::Hu);
        assert_eq!(Lang::parse("de"), Lang::En);
    }
}

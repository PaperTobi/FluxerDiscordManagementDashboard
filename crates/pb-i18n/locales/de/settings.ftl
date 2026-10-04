# Namen und Hilfetexte von Einstellungen, Abschnitten, Erkennungsarten und Auswahlwerten.
# IDs sind "setting-" plus der Schlüssel der Einstellung, "_" als "-" geschrieben.

## Abschnitte

section-tracking = Verfolgen
section-detection = Erkennung
section-warning = Warnung
section-escalation = Eskalation
section-greeting = Begrüßung
section-reporting = Berichte
section-recording = Aufnahmen
section-commands = Chat-Befehle
section-system = System

## Woher ein Wert kommt

source-builtin = eingebauter Standard
source-file = Einstellungsdatei
source-global = global
source-server = diese Community
source-person = diese Person
docs-scopes = einstellbar: { $scopes }
docs-who = ändern dürfen: { $who }
docs-default = Vorgabe: { $value }
scope-global = Global
scope-server = Community
scope-person = Person
who-admins = Admins der Community und der Bot-Besitzer
who-owner = nur der Bot-Besitzer
apply-live = gilt sofort
apply-reconnect = gilt, sobald der Bot sich neu verbindet

## Erkennungsarten

label-privacy_asking_for_pii = Fragen nach persönlichen Daten
label-discriminatory = Diskriminierung
label-harassment = Belästigung
label-sexual_content = Sexuelle Inhalte
label-illegal_and_regulated_content = Illegales und Reguliertes
label-dating_and_romantic_content = Dating und Romantik
label-profanity = Fluchen
label-disruptive_audio = Störgeräusche

setting-label-enabled = Bei { $label } warnen
    .help = Sätze zählen, die das Modell als { $label } einstuft.
setting-label-threshold = Schwelle für { $label }
    .help = Wie sicher sich das Modell sein muss (0 bis 1, niedriger ist strenger). Leer: die allgemeine Schwelle.

## Verfolgen

setting-paused = Pausiert
    .help = Den Sprachkanal verlassen und nicht mehr zuhören (für eine ganze Community oder eine Person).
setting-guild-allowlist = Nur diese Communitys
    .help = IDs der Communitys, in denen der Bot arbeitet. Leer: jede Community, in der er ist.
setting-tracked-everywhere = In allen Communitys verfolgt
    .help = Benutzer-IDs, denen der Bot in jeder gemeinsamen Community folgt.
setting-allow-e2ee-downgrade = Ende-zu-Ende-verschlüsselten Anrufen beitreten
    .help = Tritt ein Bot einem Ende-zu-Ende-verschlüsselten Anruf bei, schaltet das die Verschlüsselung für alle darin aus.
setting-join-settle = Beitrittsverzögerung
    .help = So lange warten, bevor der Bot beitritt, damit Kanalwechsel ihn nicht hin und her springen lassen.
setting-leave-grace = Verlassensverzögerung
    .help = So lange bleiben, nachdem die letzte verfolgte Person gegangen ist.

## Erkennung

setting-threshold = Allgemeine Schwelle
    .help = Wie sicher sich das Modell sein muss, bevor ein Satz zählt (0 bis 1, niedriger ist strenger).
setting-strikes = Strikes vor einer Warnung
    .help = So viele markierte Sätze im Strike-Fenster, bevor der Bot reagiert.
setting-strike-window = Strike-Fenster
    .help = Ältere markierte Sätze zählen nicht mehr als Strike. Unbegrenzt: Sie zählen immer.
setting-end-silence = Pause, die einen Satz beendet
    .help = Kürzer reagiert schneller, schneidet Sätze aber öfter ab.
setting-max-sentence = Längster Satz
    .help = Lange Rede wird nach dieser Zeit an der leisesten Stelle geschnitten.
setting-min-voiced = Kürzeste bewertete Sprache
    .help = Weniger Sprache als das in einem Satz geht nicht an das Modell.
setting-max-reaction-delay = Späteste Warnung
    .help = Wird ein Satz später als das nach seinem Ende bewertet, wird er nur protokolliert und keine Warnung mehr abgespielt. Unbegrenzt: immer warnen.

## Warnung

setting-observe-only = Nur beobachten (still)
    .help = Bewerten und protokollieren, aber nie warnen oder eingreifen.
setting-audience = Wer die Warnung hört
    .help = Nur die Person, die es gesagt hat, alle verfolgten Personen im Anruf oder alle im Kanal.
setting-volume-db = Lautstärke der Warnung
    .help = Lauter oder leiser als die Aufnahme, in Dezibel.
setting-voice-language = Gesprochene Sprache
    .help = Die Sprache, in der Warnungen gesprochen werden. Automatisch: die Sprache, die das Modell gehört hat.
setting-fallback-languages = Ersatzsprachen
    .help = Werden der Reihe nach versucht, wenn es in der gesprochenen Sprache keinen Sprachtext gibt.
setting-tts-voices = Stimmen der Sprachausgabe
    .help = Welche Stimme welche Sprache spricht.
setting-speech-rate = Sprechtempo
    .help = 1 ist normal; 1,5 ist anderthalbmal so schnell.
setting-no-speak-policy = Ohne die Berechtigung „Sprechen“
    .help = Die Warnung in den Chat des Sprachkanals schreiben oder nur protokollieren.
setting-strike-notice = Strikes ansagen
    .help = Vor der eigentlichen Warnung „Strike 1 von 3“ sagen.
setting-announce-actions = Maßnahmen ansagen
    .help = Ansagen, was der Bot tut (stummschalten, trennen, Timeout), wenn er es tut.

## Eskalation

setting-violation-window = Verstöße zählen über
    .help = Die Verstöße in dieser Zeit bestimmen die Eskalationsstufe. Unbegrenzt: Jeder Verstoß zählt.
setting-escalation = Eskalationsstufen
    .help = Ab dem n-ten Verstoß: wer benachrichtigt wird und welche Maßnahme läuft. Die Stufe wählt auch den Sprachtext.
setting-actions-enabled = Moderationsmaßnahmen erlauben
    .help = Personen stummschalten, trennen oder in den Timeout schicken, wie die Stufen es sagen. Aus: nur warnen und benachrichtigen.

## Begrüßung

setting-greet-enabled = Begrüßung
    .help = Hallo sagen, wenn sich der Bot und diese Person in einem Anruf treffen.

## Berichte

setting-modlog-channel = Mod-Log-Kanal
    .help = Textkanal, in dem jeder markierte Satz gepostet wird. Leer: aus.
setting-modlog-audio = Aufnahmen im Mod-Log
    .help = Den markierten Satz als Aufnahme an Mod-Log-Beiträge anhängen.
setting-owner-dm-audio = Aufnahmen in Nachrichten an den Besitzer
    .help = Den Satz an die Direktnachricht anhängen, die eine Eskalationsstufe dem Besitzer des Bots schickt.
setting-digest = Zusammenfassender Bericht
    .help = Eine Direktnachricht an den Bot-Besitzer mit einer Zusammenfassung pro Person.
setting-digest-time = Uhrzeit des Berichts
    .help = Wann der Bericht verschickt wird.
setting-digest-weekday = Tag des Berichts (wöchentlich)
    .help = An welchem Tag der wöchentliche Bericht verschickt wird.
setting-timezone = Zeitzone
    .help = IANA-Name, etwa Europe/Berlin.
setting-jar-enabled = Fluchkasse
    .help = Verstöße pro Person zählen (der Befehl jar und die Weboberfläche).
setting-chat-language = Chat-Sprache
    .help = Die Sprache der Chat-Antworten, Mod-Log-Beiträge und Berichte des Bots.

## Aufnahmen

setting-recordings = Aufnahmen
    .help = Welche Sätze verfolgter Personen als Aufnahme behalten werden (jede kann später gelöscht werden).
setting-admins-play-audio = Admins der Community dürfen Aufnahmen abspielen
    .help = Aus: Nur der Bot-Besitzer kann Aufnahmen anhören.

## Chat-Befehle

setting-commands-enabled = Chat-Befehle
    .help = Auf Befehle wie „!pb status“ in Textkanälen antworten.
setting-command-prefix = Befehlspräfix
    .help = Text, mit dem ein Befehl beginnt. Erwähnungen des Bots funktionieren immer.
setting-admin-user-ids = Weitere Bot-Besitzer
    .help = Diese Personen dürfen alles, was der Bot-Besitzer darf, in allen Communitys.
setting-admin-role-ids = Admin-Rollen
    .help = Mitglieder dieser Rollen dürfen den Bot in dieser Community steuern.

## System

setting-instance = Fluxer-Instanz
    .help = Die Adresse der Fluxer-API, etwa https://api.fluxer.app oder eine selbst betriebene Instanz.
setting-ui-url = Adresse der Weboberfläche
    .help = Wie diese Seite geöffnet wird, wenn nicht über die IP-Adresse, etwa http://botbox.lan:8790.
setting-allowed-hosts = Weitere Hostnamen
    .help = Andere Namen, unter denen diese Seite geöffnet werden darf.
setting-cpu-threads = CPU-Threads für das Modell
    .help = Wie viele Threads der Klassifikator auf der CPU nutzt (die GPU braucht das nicht).
setting-tts-threads = CPU-Threads für die Sprachausgabe
    .help = Wie viele Threads die Sprachausgabe nutzt.

## Auswahlwerte

choice-audience-offender = Nur die Person, die es gesagt hat
choice-audience-tracked = Verfolgte Personen
choice-audience-channel = Alle im Kanal
choice-no-speak-policy-text = In den Chat schreiben
choice-no-speak-policy-log = Nur protokollieren
choice-digest-off = Aus
choice-digest-daily = Täglich
choice-digest-weekly = Wöchentlich
choice-weekday-monday = Montag
choice-weekday-tuesday = Dienstag
choice-weekday-wednesday = Mittwoch
choice-weekday-thursday = Donnerstag
choice-weekday-friday = Freitag
choice-weekday-saturday = Samstag
choice-weekday-sunday = Sonntag
choice-recordings-off = Keine
choice-recordings-flagged = Markierte Sätze
choice-recordings-all = Jeder Satz
choice-step-action-none = Keine
choice-step-action-mute = Stummschalten
choice-step-action-disconnect = Trennen
choice-step-action-timeout = Timeout
choice-voice-language-auto = Automatisch (die gehörte Sprache)

## Warum ein Wert abgelehnt wurde

err-setting = { $setting }: { $problem }
err-unknown-setting = Es gibt keine Einstellung namens „{ $name }“.
err-scope = { $setting } lässt sich nicht { $scope ->
        [global] global
        [server] für eine Community
       *[person] für eine Person
    } setzen.
err-owner-only = Nur der Bot-Besitzer kann { $setting } ändern.
err-not-probability = { $value } liegt nicht zwischen 0 und 1
err-below-one = muss mindestens 1 sein
err-not-whole = „{ $value }“ ist keine ganze Zahl
err-too-large = „{ $value }“ ist zu groß
err-not-duration = „{ $value }“ ist keine Zeitdauer (wie 20s, 5m, 2h oder 1d)
err-negative = „{ $value }“ ist negativ
err-not-positive = muss länger als null sein
err-below-frame = muss mindestens 32 ms sein (ein Audio-Frame)
err-not-finite = muss eine endliche Zahl sein
err-not-above-zero = muss größer als null sein
err-not-time-of-day = „{ $value }“ ist keine Uhrzeit (wie 09:00)
err-unknown-tz = unbekannte Zeitzone „{ $value }“ (nutze einen Namen wie Europe/Berlin)
err-not-origin = „{ $value }“ ist keine http(s)-Adresse
err-origin-with-path = „{ $value }“ darf nur die Adresse sein, wie http://192.168.1.50:8790 (ohne Pfad)
err-not-host = „{ $value }“ ist kein Hostname
err-prefix-spaces = das Präfix darf keine Leerzeichen enthalten
err-not-lang = „{ $value }“ ist kein Sprachkürzel (wie de oder en-US)
err-not-id = „{ $value }“ ist keine ID
err-not-choice = „{ $value }“ muss eines davon sein: { $choices }
err-no-steps = die Eskalation braucht mindestens eine Stufe
err-step-order = Stufe { $step }: Stufen müssen bei steigender Verstoßzahl beginnen (1, 2, 3 …)
err-timeout-too-long = Stufe { $step }: Fluxer erlaubt Timeouts von höchstens 365,25 Tagen
err-step = Stufe { $step }: { $problem }
err-not-number = „{ $value }“ ist keine Zahl
err-not-switch = muss an oder aus sein
err-not-text = muss Text sein
err-not-list = muss eine Liste sein
err-unknown-field = unbekanntes Feld „{ $value }“

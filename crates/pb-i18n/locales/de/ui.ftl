# Die Weboberfläche.

ui-ago-now = gerade eben
ui-ago-s = vor { $n } s
ui-ago-min = vor { $n } min
ui-ago-h = vor { $n } h
ui-ago-d = { $n ->
        [one] gestern
       *[other] vor { $n } Tagen
    }

ui-station-recording = Spricht
ui-station-cut = Geschnitten
ui-station-queued = Wartet
ui-station-model = Modell
ui-station-verdict = Urteil
ui-station-decision = Entscheidung

ui-decision-clear = Unauffällig
ui-decision-invalid = Ungültige Bewertung
ui-decision-untracked = Nicht mehr verfolgt
ui-decision-strike = Strike { $strike } von { $of }
ui-decision-warn = Gewarnt (Stufe { $step })
ui-decision-observe = Beobachtet (Stufe { $step })
ui-decision-late = Zu spät zum Warnen (Stufe { $step })

ui-nav-live = Live
ui-nav-voice-lines = Sprachtexte
ui-nav-reports = Berichte
ui-nav-audit = Protokoll
ui-nav-system = System
ui-nav-communities = Communitys
ui-nav-logout = Abmelden

ui-offline = Verbindung wird wiederhergestellt …
ui-auth-expired = Deine Anmeldung ist abgelaufen.
ui-log-in-again = Erneut anmelden

ui-wall-title = Live
ui-wall-empty = Gerade hört der Bot niemandem zu. Er tritt einem Anruf bei, sobald eine verfolgte Person darin ist.
ui-wall-violations = Neueste Verstöße
ui-wall-no-violations = Noch keine Verstöße.
ui-lag = { $ms } bis zum Urteil
ui-not-in-voice = Nicht in einem Sprachkanal
ui-in-channel = In { $channel }
ui-log-in = Mit Fluxer anmelden

ui-on = An
ui-off = Aus
ui-none = Keiner
ui-save = Speichern
ui-use-inherited = Übernommenen Wert nutzen
ui-set-here = hier gesetzt
ui-inherited = von { $from }
ui-owner-only = nur Bot-Besitzer
ui-duration = Eine Dauer wie 20s, 5m oder 1h
ui-duration-or-unlimited = Eine Dauer wie 20s, 5m oder 1h, oder „unlimited“
ui-voice-default = Standardstimme
ui-voice-by-language = Stimme der Sprache
ui-esc-from = Ab Verstoß
ui-esc-action = Maßnahme
ui-esc-duration = Dauer
ui-esc-owner = Bot-Besitzer benachrichtigen
ui-esc-help = Jede Stufe gilt ab so vielen Verstößen im Zeitfenster. Fülle die leere Zeile aus, um eine Stufe hinzuzufügen; leere „Ab Verstoß“, um eine zu entfernen.
ui-saved = Gespeichert.
ui-unchanged = Nichts geändert.
ui-not-allowed = Das darfst du nicht ändern.

ui-not-found = Hier ist nichts.
ui-tab-overview = Übersicht
ui-tab-settings = Einstellungen
ui-tab-live = Live
ui-tab-history = Verlauf
ui-tab-evidence = Aufnahmen
ui-paused = Pausiert
ui-unavailable = Nicht verfügbar
ui-everywhere = Überall verfolgt
ui-not-tracked = Nicht verfolgt
ui-tracked = Verfolgt
ui-muted = stumm
ui-deafened = taub geschaltet (hört keine Warnungen)
ui-calls = Anrufe
ui-joins-paused = Der Bot wurde hier mehrmals kurz hintereinander aus dem Sprachkanal entfernt (von einem Moderator oder einer anderen Kopie des Bots?). Er tritt in { $seconds } s wieder bei und wartet nach jeder weiteren Entfernung etwas länger. Um ihn aus dieser Community herauszuhalten, pausiere ihn hier.
ui-joins-resume = Jetzt wieder beitreten
ui-joins-resumed = Der Bot tritt hier wieder bei.
ui-no-calls = Niemand ist in einem Anruf.
ui-bot-listens = der Bot hört zu
ui-bot-cannot-speak = der Bot darf nicht sprechen
ui-encrypted = Ende-zu-Ende-verschlüsselt
ui-tracked-people = Verfolgte Personen
ui-nobody-tracked = Hier wird noch niemand verfolgt. Nutze unten „Jemanden verfolgen“ (ein Name, eine Benutzer-ID oder @Erwähnung) oder den Chatbefehl add, zum Beispiel !pb add @name.
ui-track = Verfolgen
ui-untrack = Nicht mehr verfolgen
ui-track-someone = Jemanden verfolgen
ui-user-id-or-mention = Ein Name, eine Benutzer-ID oder @Erwähnung
ui-track-help = Füge eine Benutzer-ID ein (oder eine aus Fluxer kopierte Erwähnung). Der Bot hört verfolgten Personen zu, sobald sie in einem Anruf sind, dem er beitreten kann.
ui-violations = Verstöße
ui-when = Wann
ui-who = Wer
ui-decision = Entscheidung
ui-older = Ältere
ui-jar = Fluchkasse
ui-jar-empty = Die Fluchkasse ist leer.
ui-jar-reset = Leeren
ui-digest = Bericht
ui-digest-help = Der Bericht fasst die Verstöße seit dem letzten zusammen und geht an den Bot-Besitzer (täglich oder wöchentlich, siehe Berichte).
ui-digest-send = Bericht jetzt senden
ui-now = Jetzt
ui-today = Heute
ui-in-window = Im Zeitfenster
ui-in-window-value = { $n } (in { $window })
ui-next-step = Stufe des nächsten Verstoßes
ui-observe-only = Nur beobachten (keine Warnungen)
ui-said-and-done = Gesagt und getan
ui-nothing-yet = Noch nichts.
ui-conveyor = Sätze
ui-no-sentences = Noch keine Sätze.
ui-no-evidence = Keine Aufnahmen. Welche Sätze ihre Aufnahme behalten, steht unter Einstellungen → Aufnahmen.
ui-failed = fehlgeschlagen
ui-dropped-short = zu wenig Sprache
ui-dropped-echo = über der eigenen Stimme des Bots
ui-play-warning = Gewarnt
ui-play-strike = Strike-Hinweis
ui-play-action = Maßnahme angekündigt
ui-play-greeting = Begrüßt
ui-play-say = Gesagt
ui-say-now = Jetzt sagen
ui-say-placeholder = Was der Bot sagen soll
ui-their-language = Sprache der Person
ui-say = Sagen
ui-say-help = Der Bot sagt es in dem Anruf, in dem die Person ist, für das unter Warnung eingestellte Publikum: einen Text in einer Sprache, einen Clip aus der Bibliothek oder eine Sag-Vorlage aus den Sprachtexten.
ui-days = Die letzten zwei Wochen
ui-day = Tag
ui-sentences = Sätze
ui-flagged = Markiert
ui-speech = Sprechzeit
ui-length = Länge
ui-scores = Werte
ui-delete-recording = Aufnahme löschen
ui-built-in = eingebaut
ui-preview = Vorhören
ui-remove = Entfernen
ui-add-text = Text hinzufügen
ui-add-clip = Clip hinzufügen
ui-clip-removed = (entfernter Clip)
ui-vl-help = Was der Bot sagt. Ein Sprachtext hat Clips und Texte pro Sprache; der Bot nimmt einen Clip in der nötigen Sprache, sonst spricht er den Text, sonst fragt er die nächste Ebene (Person, Community, global, eingebaut). In einem Text ist { "{" }name{ "}" } der Name der Person; außerdem gibt es { "{" }label{ "}" } (was gehört wurde), { "{" }count{ "}" } (Verstöße bisher), { "{" }step{ "}" }, { "{" }strikes{ "}" }, { "{" }duration{ "}" } (einer Stummschaltung oder Auszeit), { "{" }server{ "}" } und { "{" }channel{ "}" }.
ui-vl-warning = Warnung
ui-vl-any-type = jede Art
ui-vl-any-step = jede Stufe
ui-vl-step = Stufe { $n }
ui-vl-greeting = Begrüßung
ui-vl-kind-action = Maßnahmen-Hinweise
ui-vl-kind-say = Sag-Vorlagen
ui-vl-strike = Strike-Hinweis
ui-vl-action-any = Maßnahmen-Hinweis (jede Maßnahme)
ui-vl-action = Maßnahmen-Hinweis: { $action }
ui-vl-say = Sag-Vorlage „{ $name }“
ui-vl-say-new = Eine neue Sag-Vorlage
ui-vl-name = Wie der Name gesagt wird
ui-vl-text-placeholder = Text ({"{"}name{"}"} für den Namen)
ui-vl-step-placeholder = Stufe (leer = jede Stufe)
ui-vl-preset-placeholder = Name der Vorlage (für Sagen)
ui-vl-add-line = Sprachtext hinzufügen
ui-clips = Clips
ui-clip-name = Name
ui-clip-no-speech = Keine Sprache (jede Sprache)
ui-clip-transcript = Abschrift (optional)
ui-clip-heard = klingt nach { $lang }
ui-clip-sounds-like = klingt nach { $label } ({ $score })
ui-no-clips = Noch keine Clips. Lade eine Audiodatei hoch oder nimm einen auf.
ui-upload = Hochladen
ui-record = Clip aufnehmen
ui-record-stop = Beenden und hochladen
ui-record-uploading = Wird hochgeladen …
ui-record-hint = Aufnahme läuft … zum Beenden erneut drücken.
ui-all-kinds = Alles
ui-filter = Anzeigen
ui-all-communities = Alle Communitys
ui-recording-kept = Aufnahme aufbewahrt
ui-heard-language = gehört: { $lang }
audit-by-bot = Der Bot
audit-by-file = Eine Änderung der Einstellungsdatei
audit-by-old-bot = Der alte Bot
audit-globally = global
audit-in = in { $community }
audit-for = für { $person } in { $community }
audit-set = { $by } hat { $setting } { $where } auf { $value } gesetzt
audit-clear = { $by } hat { $setting } { $where } zurückgesetzt
audit-track = { $by } verfolgt jetzt { $person } in { $community }
audit-untrack = { $by } verfolgt { $person } in { $community } nicht mehr
audit-voice-line = { $by } hat den Sprachtext „{ $line }“ { $where } geändert
audit-imported = Alter Bot: { $by } hat { $setting } auf { $value } gesetzt
audit-action = { $action } für { $person } in { $community }: { $result }
audit-jar-reset = { $by } hat die Fluchkasse von { $person } in { $community } geleert
audit-jar-baseline = Die Fluchkasse von { $person } in { $community } beginnt bei { $count } (vom alten Bot)
audit-recording-deleted = { $by } hat eine Aufnahme gelöscht
audit-clip-saved = { $by } hat den Clip „{ $name }“ gespeichert
audit-clip-removed = { $by } hat einen Clip entfernt
audit-voice-saved = { $by } hat die Stimme „{ $name }“ gespeichert
audit-voice-removed = { $by } hat die Stimme { $voice } entfernt
audit-login = { $name } hat sich angemeldet
audit-msg-modlog = Mod-Log-Beitrag
audit-msg-dm = Nachricht an den Bot-Besitzer
audit-msg-digest = Bericht
audit-message-sent = { $what } gesendet
audit-message-failed = { $what } konnte nicht gesendet werden: { $error }
audit-import = Importiert aus { $from }: { $sentences } Sätze, { $recordings } Aufnahmen, { $clips } Clips, { $settings } Einstellungen
audit-log-repaired = Das Ereignisprotokoll wurde repariert: { $bytes } nach Ereignis { $seq } abgeschnitten
audit-started = Der Bot ist gestartet (Version { $version })
audit-stopped = Der Bot wurde beendet
audit-stopped-unclean = Der Bot wurde beendet, bevor jeder Satz bewertet war
audit-unknown = { $kind } (von einer neueren Version geschrieben)
audit-group-settings = Einstellungen
audit-group-actions = Moderationsmaßnahmen
audit-group-jar = Fluchkasse
audit-group-library = Clips und Aufnahmen
audit-group-logins = Anmeldungen
audit-group-messages = Nachrichten
audit-group-bot = Der Bot
ui-status = Status
ui-version = Version
ui-microphones = Mikrofone
ui-models = Modelle
ui-ready = bereit
ui-queues = Warteschlangen
ui-waiting = Wartend
ui-done = Erledigt
ui-oldest = Älteste
ui-parts = Teile des Bots
ui-restarts = Neustarts
ui-part-running = läuft
ui-part-restarting = startet neu
ui-part-not-answering = antwortet nicht
ui-part-stopped = gestoppt
ui-part-failed = ausgefallen (der Bot startet neu)
ui-part-moderation = Entscheidungen
ui-part-undo = Stummschaltungen aufheben
ui-part-digest = Zusammenfassungen
ui-part-threads = Modell-Threads
ui-part-views = Live-Seiten
ui-part-gateway = Fluxer-Verbindung
ui-part-system = Diese Seite
ui-storage = Speicher
ui-log = Ereignisprotokoll
ui-files = Dateien
ui-free = Frei
ui-index-behind = Index im Rückstand
ui-fluxer-no-token = kein Bot-Token
ui-fluxer-connecting = verbindet
ui-fluxer-ready = online als { $bot }
ui-fluxer-reconnecting = verbindet erneut ({ $error })
ui-fluxer-no-voice = diese Fluxer-Instanz hat Sprachkanäle abgeschaltet
ui-fluxer-rejected = der Token wurde abgelehnt
ui-fluxer-secrets = Fluxer und Geheimnisse
ui-secrets-help = Ein Geheimnis zu ersetzen braucht eine Anmeldung aus den letzten 15 Minuten. Mit einem neuen Token meldet sich der Bot sofort neu an.
ui-client-secret-help = Aus derselben Anwendung in Fluxer (Benutzereinstellungen → Anwendungen). Anmeldungen an dieser Weboberfläche nutzen es.
ui-replace = Ersetzen
ui-reconnect = Neu mit Fluxer verbinden
ui-reload-settings = Einstellungs- und Stimmdateien neu lesen
ui-clip-added = Der Clip „{ $name }“ wurde hinzugefügt.
ui-clip-removed-notice = Der Clip wurde aus der Bibliothek entfernt. Sprachtexte, die ihn nutzten, überspringen ihn.
ui-clip-unreadable = Diese Datei lässt sich nicht als Clip verwenden: { $error }
ui-digest-not-sent = Der Bericht konnte nicht zugestellt werden (siehe unten).
ui-digest-sent = Der Bericht wurde gesendet.
ui-jar-emptied = Die Fluchkasse wurde geleert.
ui-no-longer-tracking = { $name } wird nicht mehr verfolgt.
ui-no-such-clip = Dieser Clip ist nicht in der Bibliothek.
ui-not-a-line = Das ist kein Sprachtext.
ui-not-a-user = Das ist keine Benutzer-ID.
ui-now-tracking = { $name } wird jetzt verfolgt.
ui-reconnected = Wieder mit Fluxer verbunden.
ui-recording-deleted = Die Aufnahme wurde gelöscht.
ui-reloaded = Die Einstellungs- und Stimmdateien wurden neu gelesen.
ui-said = Gesagt.
ui-say-empty = Schreib, was der Bot sagen soll.
ui-token-replaced = Der neue Token funktioniert; der Bot ist online.
ui-upload-empty = Wähle eine Datei zum Hochladen.
ui-upload-failed = Das Hochladen brach ab. Bitte versuche es erneut.
ui-secret-from-env = In der Umgebung des Bots gesetzt (PB_BOT_TOKEN / PB_CLIENT_SECRET, zum Beispiel ein Podman-Secret); dort ändern.

perm-view-channel = Kanal ansehen
perm-send-messages = Nachrichten senden
perm-attach-files = Dateien anhängen
perm-add-reactions = Reaktionen hinzufügen
perm-read-message-history = Nachrichtenverlauf lesen
perm-connect = Mit Sprachchat verbinden
perm-speak = Sprechen
perm-mute-members = Mitglieder stummschalten
perm-move-members = Mitglieder verschieben
perm-moderate-members = Timeouts für Mitglieder verhängen
perm-manage-guild = Community verwalten
perm-administrator = Administrator
ui-permissions = Was der Bot hier darf
ui-perm-ok = alles Nötige
ui-perm-missing = fehlt: { $missing }
ui-perm-modlog = Mod-Log #{ $channel }
ui-perm-actions = Moderationsmaßnahmen (sie sind an)
ui-invite = + Bot einladen
ui-invite-help = Einladen fügt den Bot einer Community auf Fluxer hinzu. Wer den Link öffnet, braucht dort „Community verwalten“: Fluxer fragt nach der Community und zeigt die Berechtigungen, um die der Bot bittet. Danach erscheint der Bot in der Seitenleiste; einem Anruf tritt er erst bei, wenn jemand Verfolgtes darin ist.
ui-clip-not-yours = Nur wer diesen Clip hinzugefügt hat oder der Besitzer des Bots kann ihn ändern oder entfernen.
ui-clip-added-by = hinzugefügt von { $name }
ui-track-the-bot = Der Bot verfolgt sich nicht selbst.
ui-tracked-everywhere = { $name } wird in jeder Community verfolgt; das beendet der Besitzer auf der Seite der Person oder auf der Systemseite.
ui-digest-last-sent = Der letzte Bericht hat den Besitzer erreicht (er reichte bis { $when }).
ui-digest-failed = Der letzte Bericht konnte nicht zugestellt werden: { $error }. Fluxer lehnt Direktnachrichten ab, wenn der Besitzer sie von Bots nicht annimmt; „Bericht jetzt senden“ versucht es erneut.
ui-logout-everywhere = Auf allen Geräten abmelden
ui-logout-everywhere-help = Auf allen Geräten abmelden (wenn eine Anmeldung in falsche Hände geraten sein könnte).
ui-permissions-help = Fehlende Berechtigungen gibt man der Rolle des Bots in den Community-Einstellungen von Fluxer (Rollen), oder für einen Kanal in dessen Berechtigungen.
ui-say-own-text = Text
ui-voices = Stimmen
ui-voices-help = Piper-Stimmen für die Sprachausgabe. Für eine weitere Sprache die beiden Dateien einer Stimme (.onnx und .onnx.json, aus der Sammlung der Piper-Stimmen) in den Ordner voices der Bot-Daten legen (/data/voices im Container) und die Dateien neu einlesen.
ui-record-needs-https = Aufnehmen geht nur, wenn die Weboberfläche über HTTPS (oder auf localhost) läuft; lade stattdessen eine Datei hoch.
ui-record-refused = Der Browser hat das Mikrofon nicht erlaubt.
ui-record-upload-failed = Die Aufnahme konnte nicht hochgeladen werden
ui-pause-here = Hier pausieren
ui-resume-here = Hier fortsetzen
err-not-connected = Der Bot ist gerade nicht mit Fluxer verbunden.
err-not-in-call = Die Person ist in keinem Anruf.
err-bot-not-in-call = Der Bot ist nicht im Anruf der Person.
err-no-language = Für die Person ist keine Sprache eingestellt; wähle eine.
err-said-too-late = Es hat zu lange gewartet und wurde nicht gesagt.
err-said-not-spoken = Der Bot darf in diesem Anruf nicht sprechen, deshalb wurde es nicht gesagt.
err-said-nothing = Es gab nichts zu sagen: kein Clip, kein Text und keine Stimme für die Sprache der Person.
err-said-failed = Es konnte nicht gesagt werden: { $error }
err-no-such-sentence = Diesen Satz gibt es nicht.
err-no-recording = Dieser Satz hat keine Aufnahme.
err-no-such-voice = Diese Stimme ist nicht (mehr) in der Sammlung.
err-voice-no-cloning = Dieses Sprachmodell kann keine Stimmen aus Aufnahmen machen.
err-voice-needs-transcript = Dieses Sprachmodell muss wissen, was in der Aufnahme gesagt wird: bitte eintippen.
err-voice-no-model = Das Sprachmodell { $model } läuft nicht (seine Dateien fehlen, oder es konnte nicht starten; siehe die Seite System).
err-voice = Die Stimme konnte nicht gemacht werden: { $error }
err-log-halted = Das Ereignisprotokoll schreibt nicht (siehe System), daher wurde nichts geändert.
err-render = Die Sprachausgabe konnte nicht erzeugt werden: { $error }
err-render-no-voice = Keine installierte Stimme spricht { $lang }.
err-render-clip-missing = Ein Clip dieser Ansage fehlt im Datenverzeichnis.
err-fluxer = Fluxer meldet: { $error }
err-bad-instance = Das ist keine Adresse einer Fluxer-Instanz: { $error }
err-store = Die Daten konnten nicht gelesen oder geschrieben werden: { $error }
err-unknown-host = Diese Adresse ist keine der Adressen der Weboberfläche. Öffne sie über die IP-Adresse und trage diesen Namen dann unter System → Adresse der Weboberfläche oder Weitere Hostnamen ein.
err-bad-ui-address = die Einstellung „Adresse der Weboberfläche“ ist keine brauchbare Adresse
err-no-login-code = Fluxer hat keinen Anmeldecode zurückgeschickt
ui-fluxer-stopped = endgültig gestoppt: { $error } (der Bot braucht ein Update oder eine neue Einrichtung)
ui-model-classifier = Klassifikator (Roblox Voice Safety)
ui-model-voice-activity = Spracherkennung (Silero)
ui-model-speech = Sprachausgabe (Piper)
ui-model-not-answering = antwortet nicht: Bot neu starten
ui-model-no-voices = keine Stimme installiert
ui-queue-scoring = Zu bewertende Sätze
ui-queue-speech = Zu erzeugende Sprache
ui-log-halted = Das Ereignisprotokoll schreibt nicht mehr: { $error }. Bis es wieder schreibt, wird nichts aufgezeichnet: Speicherplatz freigeben und erneut versuchen.
ui-log-retry = Erneut schreiben
ui-log-writing-again = Das Ereignisprotokoll schreibt wieder.
ui-index-problem = Der Suchindex kommt nicht hinterher ({ $error }); er versucht es weiter. Den Seiten fehlen so lange die neuesten Ereignisse.
ui-index-skipped = { $count ->
    [one] Ein Ereignis im Protokoll stammt von einer neueren Version des Bots und fehlt auf den Seiten.
   *[other] { $count } Ereignisse im Protokoll stammen von einer neueren Version des Bots und fehlen auf den Seiten.
}
ui-bot-joining = der Bot tritt bei
ui-bot-retrying = Beitritt fehlgeschlagen; der Bot versucht es gleich erneut
ui-bot-leaving = der Bot verlässt den Kanal
ui-source = Quellcode (AGPL-3.0)

## Den ganzen Bot pausieren
ui-paused-everywhere = Überall pausiert
ui-paused-everywhere-banner = Der Bot ist überall pausiert: Er hört niemandem zu und sagt nichts, bis der Besitzer des Bots ihn wieder einschaltet.
ui-paused-everywhere-where = Wieder einschalten (System)
ui-pause-everywhere-title = Der Bot läuft
ui-pause-everywhere-help = Pausieren hält ihn überall auf einmal an: Er verlässt jeden Anruf, hört niemandem zu und sagt nichts, bis du ihn wieder einschaltest. Einstellungen und verfolgte Personen bleiben, wie sie sind.
ui-pause-everywhere = Überall pausieren
ui-resume-everywhere-help = Der Bot hört niemandem zu und sagt nichts. Pausen einzelner Communitys und Personen bleiben bestehen, wenn er wieder läuft.
ui-resume-everywhere = Überall fortsetzen

## Einladen
ui-invite-title = Bot einladen
ui-invite-what = Was Einladen bewirkt
ui-invite-not-connected = Der Bot ist noch nicht mit Fluxer verbunden, deshalb kann er weder den Einladungslink erstellen noch seine Communitys sehen (warum, sieht der Besitzer des Bots auf der Systemseite).
ui-invite-permissions = Der Link bittet um das, was der Bot braucht: Kanäle sehen und darin schreiben (für das Mod-Log), Sprachkanälen beitreten und sprechen. Solange Moderationsmaßnahmen in irgendeiner Community an sind, bittet er auch darum, Mitglieder stummzuschalten, zu verschieben und ihnen eine Auszeit zu geben.
ui-invite-open = In Fluxer öffnen
ui-invite-no-link = Der Link erscheint, sobald der Bot sein Token hat und seine Fluxer-Instanz erreicht.
ui-reauthorize-title = Communitys, in denen dem Bot Berechtigungen fehlen
ui-reauthorize-help = Erneutes Autorisieren in Fluxer gibt der Rolle des Bots, was fehlt (wer das tut, braucht dort „Community verwalten“). Berechtigungen, die in einem einzelnen Kanal entzogen wurden, gibt man in den Einstellungen dieses Kanals zurück.
ui-reauthorize = Erneut autorisieren
ui-copy = Kopieren
ui-copied = Kopiert

## Rückfragen
ui-cancel = Abbrechen
ui-back = Zurück
ui-confirm-untrack = { $name } nicht mehr verfolgen?
ui-confirm-untrack-what = Der Bot hört { $name } in { $community } nicht mehr zu und verlässt den Anruf, außer er folgt dort noch jemand anderem. Was er über die Person behalten hat (Verlauf, Aufnahmen, Fluchkasse), bleibt; wird sie wieder verfolgt, geht es dort weiter.
ui-confirm-jar = Die Fluchkasse von { $name } leeren?
ui-confirm-jar-what = { $count ->
        [one] Sie enthält einen Verstoß in { $community }.
       *[other] Sie enthält { $count } Verstöße in { $community }.
    } Sie beginnt wieder bei 0; der Verlauf des Gesagten bleibt.
ui-jar-reset-button = Fluchkasse leeren
ui-confirm-recording = Diese Aufnahme löschen?
ui-confirm-recording-what = Die Aufnahme von { $name } in { $community } vom { $when } wird endgültig gelöscht und lässt sich nicht zurückholen. Der Satz, seine Werte und die Entscheidung bleiben im Verlauf.
ui-confirm-clip = Den Clip „{ $name }“ entfernen?
ui-confirm-clip-what = Diese Sprachtexte nutzen ihn. Sie überspringen ihn ab jetzt; ein Sprachtext ohne weitere Clips oder Texte fällt auf die nächste Ebene zurück (Community, global, eingebaut).
ui-confirm-clip-button = Clip entfernen
ui-confirm-clip-elsewhere = { $count ->
        [one] ein Sprachtext in einer Community, die du nicht verwaltest
       *[other] { $count } Sprachtexte in Communitys, die du nicht verwaltest
    }

## Einstellungsformulare
ui-help = Was das bewirkt
ui-advanced = Erweitert
ui-advanced-help = Einstellungen, die selten geändert werden müssen: wie Sprache in Sätze geschnitten wird, Stimmen und Sprechtempo und das Innenleben des Bots.
ui-reset-settings = Alle Einstellungen hier zurücksetzen
ui-reset-settings-help = { $count ->
        [one] Hier ist eine Einstellung gesetzt.
       *[other] Hier sind { $count } Einstellungen gesetzt.
    } Zurücksetzen lässt sie alle wieder übernehmen.
ui-settings-reset = { $count ->
        [one] Eine Einstellung wurde zurückgesetzt.
       *[other] { $count } Einstellungen wurden zurückgesetzt.
    }
ui-confirm-reset = Alle Einstellungen hier zurücksetzen?
ui-confirm-reset-what = Diese Einstellungen ({ $where } gesetzt) werden entfernt und übernehmen ihre Werte wieder aus { $from }:
ui-confirm-reset-from-defaults = der Einstellungsdatei oder den eingebauten Standardwerten
ui-confirm-reset-from-global = den globalen Einstellungen
ui-confirm-reset-from-community = den Einstellungen der Community
ui-confirm-reset-nothing = Hier ist nichts gesetzt, was du zurücksetzen darfst.
ui-confirm-reset-kept = Pausen und die Systemeinstellungen (die Fluxer-Instanz, die Adresse der Weboberfläche) bleiben, wie sie sind.
ui-works-with = Wirkt zusammen mit „{ $setting }“, das { $where } eingestellt wird:
ui-where-per-community = pro Community
ui-where-globally = global auf der Seite System
ui-where-per-person = pro Person
ui-not-set = nicht gesetzt

## Jetzt sagen, Vorhören
ui-say-clip = Clip
ui-say-preset = Vorlage
ui-say-play = Abspielen
ui-say-language = Sprache
ui-say-bot-not-in-call = Der Bot ist nicht in ihrem Anruf: Er tritt den Anrufen verfolgter Personen bei (außer sie oder die Community sind pausiert).
ui-preview-language = Die Sprache zum Vorhören
ui-preview-as-bot = So, wie der Bot es sagen würde

## Erste Schritte, Clipnamen
ui-no-communities = Der Bot ist noch in keiner Community.
ui-first-invite = Der Bot ist noch in keiner Community. Lade ihn zuerst in eine Fluxer-Community ein; wer dort „Community verwalten“ darf, kann ihn hinzufügen.
ui-first-track = Noch wird niemand verfolgt. Öffne eine Community und verfolge jemanden: Der Bot hört der Person zu, sobald sie in einem Anruf ist, dem er beitreten kann.
ui-clip-name-empty = Ein Clip braucht einen Namen.
ui-record-default-name = Aufnahme { $when }

## Chat-Befehle
ui-chat-commands = Chat-Befehle
ui-chat-commands-how = Fluxer hat keine Slash-Befehle und keine Autovervollständigung: Ein Befehl ist eine gewöhnliche Nachricht in einem Textkanal der Community, die mit { $prefix } oder einer Erwähnung des Bots beginnt; er antwortet in diesem Kanal.
ui-chat-commands-off = Chat-Befehle sind ausgeschaltet (System → Chat-Befehle).

## Listen von Communitys, Rollen und Personen
ui-list-empty = Noch keine.
ui-list-add = Hinzufügen
ui-list-add-community = Name oder ID einer Community
ui-list-add-role = Name oder ID einer Rolle
ui-list-add-empty = Gib einen Namen ein oder füge eine ID ein.
ui-list-added = { $name } hinzugefügt.
ui-list-added-unknown = { $name } hinzugefügt (der Bot hat diese ID noch nicht gesehen).
ui-list-already = { $name } steht schon auf der Liste.
ui-list-removed = { $name } entfernt.
ui-list-not-there = { $name } steht nicht auf der Liste.
ui-list-no-match = Nichts namens „{ $name }“ ist bekannt. Füge stattdessen die ID ein.
ui-list-several = Mehrere passen zu „{ $name }“: { $matches }. Gib mehr vom Namen ein oder füge die ID ein.
ui-confirm-list-remove = { $name } von „{ $setting }“ entfernen?
ui-list-remove-what = { $name } wird von der Liste genommen.
ui-list-remove-guild-allowlist = Der Bot arbeitet nicht mehr in { $name }: Er verlässt dort seine Anrufe und beantwortet keine Befehle mehr.
ui-list-remove-last-community = Es ist die letzte Community auf der Liste: Mit leerer Liste arbeitet der Bot in jeder Community, in der er ist.
ui-list-remove-tracked-everywhere = Der Bot folgt { $name } nicht mehr in jeder Community; wo eine Community die Person selbst verfolgt, tut er es weiter.
ui-list-remove-admin-user-ids = { $name } ist kein Besitzer des Bots mehr; die Anmeldungen der Person verlieren die Rechte des Besitzers.
ui-list-remove-admin-role-ids = Wer die Rolle { $name } hat, verwaltet den Bot nicht mehr (außer die Person verwaltet die Community ohnehin).
ui-track-everywhere = In jeder Community verfolgen
ui-untrack-everywhere = Nicht mehr in jeder Community verfolgen
ui-everywhere-hint = Wer in jeder Community verfolgt wird, wird auf der eigenen Seite oder auf der Seite System von dieser Liste genommen.

## Pausieren, mit Ort
ui-paused-done = Pausiert { $where }: Der Bot verlässt dort den Sprachkanal, hört niemandem zu und sagt nichts.
ui-resumed-done = Fortgesetzt { $where }: Der Bot folgt dort den verfolgten Personen wieder.
ui-in-every-community = in jeder Community

## Namen der Teile des Bots, Personen nach ID
ui-part-recorder = Ereignisprotokoll schreiben
ui-part-enforcer = Maßnahmen und Nachrichten
audit-by-id = jemand (ID { $id })

## Sprachtexte: woher ein Sprachtext kommt
ui-vl-own = Eigener Sprachtext
ui-vl-uses = nutzt „{ $line }“ ({ $from })

## Was Speichern bewirkt hat
ui-saved-what = Gespeichert: { $what }.

## Weitere Rückfragen
ui-confirm-line-clear = Die eigenen Clips und Texte von „{ $line }“ entfernen?
ui-confirm-line-clear-what = Der Sprachtext verliert { $where } { $clips ->
        [one] seinen Clip
       *[other] seine { $clips } Clips
    } und { $texts ->
        [one] seinen Text
       *[other] seine { $texts } Texte
    }. Der Bot sagt dann, was der Sprachtext weiter oben sagt (oder den eingebauten Text).
ui-confirm-setting-clear = „{ $setting }“ zurücksetzen?
ui-confirm-setting-clear-what = Es gilt wieder { $value } ({ $from }).
ui-confirm-setting-clear-reconnect = Der Bot verbindet sich damit neu mit Fluxer.
ui-confirm-logout-all = Auf allen Geräten abmelden?
ui-confirm-logout-all-what = Jede deiner Anmeldungen endet, auf jedem Gerät, auch diese. Nutze das, wenn jemand eine Anmeldung übernommen haben könnte.

# Text, den der Bot in Chats und Direktnachrichten schreibt.
# Erwähnungen (<@id>, <#id>) und vorformatierte Zahlen kommen als fertige Zeichenketten an.

## Chat-Befehle

cmd-help = **Profanity Watch** · Präfix `{ $prefix }` oder erwähne mich
    Alle: `{ $prefix } status` · `{ $prefix } list` · `{ $prefix } jar [@user]` · `{ $prefix } help`
    Admins (Community verwalten, Administrator oder eine Admin-Rolle):
    `{ $prefix } add @user…` / `{ $prefix } remove @user…`: diesen Personen in dieser Community in den Sprachkanal folgen
    `{ $prefix } pause` / `{ $prefix } resume`: hier nicht mehr folgen oder wieder folgen
    `{ $prefix } observe on|off`: nur bewerten und protokollieren, nie warnen
    `{ $prefix } set threshold 0.6 [@user]`: niedriger ist strenger; mit @user nur für diese Person
    `{ $prefix } set strikes 2` · `set window 20s` · `set audience offender|tracked|channel` · `set language de`
    `{ $prefix } set <Einstellung> <Wert> [@user]` · `{ $prefix } reset <Einstellung|all> [@user]`
    `{ $prefix } modlog #kanal` / `{ $prefix } modlog off`: jeden markierten Satz dort posten{ $audio ->
        [yes] {" "}(mit Aufnahme)
       *[no] {""}
    }
cmd-help-ui = Alles andere steht in der Weboberfläche: { $url }
cmd-help-no-ui = Alles andere steht in der Weboberfläche.

cmd-dm-only = Ich nehme Befehle nur in einem Kanal einer Community an, wo ich prüfen kann, wer Admin ist.
cmd-roles-loading = Ich lade die Rollen dieser Community noch; versuch es in ein paar Sekunden noch einmal.
cmd-denied = Nur Admins der Community (Community verwalten, Administrator oder eine Admin-Rolle) können das ändern.
cmd-unknown = Unbekannter Befehl `{ $name }`. Versuch `{ $prefix } help`.
cmd-store-failed = Geändert, aber das Speichern ist fehlgeschlagen: { $error }
cmd-usage = Aufruf: `{ $prefix } { $usage }`
cmd-no-users = Nenne mindestens eine Person: eine Erwähnung wie @name oder eine numerische Benutzer-ID.
cmd-too-many-users = Nach dem Wert ist höchstens eine Person erlaubt.
cmd-unknown-setting = Unbekannte Einstellung `{ $name }`. Zum Beispiel: threshold, strikes, window, audience, language.
cmd-web-only = { $setting } wird in der Weboberfläche bearbeitet.
cmd-no-channel = Nenne einen Kanal wie #mod-log.
cmd-channel-unknown = Diesen Kanal sehe ich in dieser Community nicht.
cmd-channel-not-text = In diesem Kanal kann man keine Nachrichten schreiben.
cmd-missing-permissions = ⚠️ Mir fehlt dort { $permissions }; gib mir das, sonst schlagen die Beiträge fehl.

cmd-list-empty = In dieser Community wird niemand verfolgt. Ein Admin kann mit `{ $prefix } add @user` jemanden hinzufügen.
cmd-list-head = **Verfolgt in dieser Community** ({ $count }){ $paused ->
        [yes] {" "}· pausiert
       *[no] {""}
    }
cmd-list-everywhere = • { $user } (global festgelegt, in allen Communitys)
cmd-list-person = • { $user }
cmd-list-note-threshold = {" "}· Schwelle { $threshold }

cmd-status-head = **Status in dieser Community** (ID { $guild })
cmd-status-paused = ⏸️ Pausiert
cmd-status-following = Folgt { $count ->
        [one] einer Person
       *[other] { $count } Personen
    }{ $people ->
        [none] {""}
       *[other] : { $people }
    }
cmd-status-mode-observe = Modus: nur beobachten (still)
cmd-status-mode-warn = Modus: warnt, wenn er etwas hört
cmd-status-detection = Schwelle { $threshold } · { $strikes ->
        [one] ein Strike
       *[other] { $strikes } Strikes
    } in { $window } · zu hören für { $audience ->
        [offender] nur die Person, die es gesagt hat
        [tracked] nur verfolgte Personen
       *[channel] alle im Kanal
    }
cmd-status-modlog-off = Mod-Log: aus
cmd-status-modlog = Mod-Log: { $channel }{ $audio ->
        [yes] {" "}(mit Aufnahme)
       *[no] {" "}(nur Text)
    }
cmd-status-room = Im Sprachkanal: { $channel }{ $people ->
        [none] {""}
       *[other] , hört { $people } zu
    }
cmd-status-model = Modell: { $ready ->
        [yes] bereit
       *[no] antwortet nicht (Bot neu starten)
    }{ $device ->
        [none] {""}
       *[other] {" "}({ $device })
    }

cmd-jar-off = Die Fluchkasse ist hier ausgeschaltet.
cmd-jar-person = { $user } hat { $count } in der Fluchkasse.
cmd-jar-empty = Die Fluchkasse ist leer. 🎉
cmd-jar-head = **Fluchkasse**
cmd-jar-line = { $rank }. { $user }: { $count }

cmd-add-self = Mich selbst kann ich nicht verfolgen.
cmd-add-done = Ich folge jetzt { $people } in die Sprachkanäle dieser Community.
cmd-add-already = Schon verfolgt: { $people }.
cmd-remove-done = Ich folge { $people } hier nicht mehr.
cmd-remove-everywhere = { $people } { $count ->
        [one] wird
       *[other] werden
    } global in allen Communitys verfolgt; das kann nur der Bot-Besitzer ändern (Weboberfläche, globale Einstellungen).
cmd-remove-missing = Hier nicht verfolgt: { $people }.
cmd-pause = In dieser Community pausiert: Ich verlasse den Sprachkanal und höre nicht mehr zu. Mit `{ $prefix } resume` geht es weiter.
cmd-resume = Fortgesetzt: Ich folge den verfolgten Personen wieder.
cmd-observe-on = Nur beobachten: Ich bewerte und protokolliere, warne und handle aber nie.
cmd-observe-off = Nur beobachten ist aus: Ich warne wieder.
cmd-set-community = { $setting } ist für diese Community jetzt { $value }.
cmd-set-person = { $setting } ist für { $user } jetzt { $value }.
cmd-reset-community = { $setting } ist für diese Community wieder auf dem Standard.
cmd-reset-person = { $setting } ist für { $user } wieder auf dem Standard.
cmd-reset-all-community = Alle Einstellungen dieser Community sind wieder auf dem Standard.
cmd-reset-all-person = Alle Einstellungen von { $user } sind wieder auf dem Standard.
cmd-modlog-off = Mod-Log aus.
cmd-modlog-set = Mod-Log ist jetzt { $channel }: Jeder markierte Satz wird dort gepostet{ $audio ->
        [yes] {" "}mit seiner Aufnahme.
       *[no] {" "}(nur Text; Aufnahmen sind für den Mod-Log nicht eingeschaltet).
    }

## Mod-Log, Direktnachrichten und der Bericht

modlog-flagged = 🔊 Markiert: { $user } in { $channel }: { $labels } · { $seconds } s · { $language } → { $decision ->
        [warn] gewarnt
        [observe] beobachtet (still)
        [late] zu spät zum Warnen
        [strike] Strike { $strike } von { $of }
       *[other] protokolliert
    }
modlog-chat-flagged = 💬 Markiert: { $user } in { $channel }: { $found } → { $decision ->
        [warn] gewarnt
        [observe] beobachtet (still)
        [strike] Strike { $strike } von { $of }
       *[other] protokolliert
    }
    > { $quote }
violation-chat = 💬 **{ $user }** in { $community } / { $channel } schrieb { $found }; Verstoß { $count } in { $window }, Stufe { $step } ({ $decision ->
        [warn] gewarnt
       *[observe] nur beobachtet, still
    })
    > { $quote }
chat-warning = bitte achte auf deine Sprache.
chat-delete-reason = Profanity Watch: ein Wort aus der Wortliste
modlog-label-score = { $label } **{ $score }** (Schwelle { $bar })
upload-failed = (Hochladen der Aufnahme fehlgeschlagen: { $error })

violation = 🔊 **{ $user }** in { $community } / { $channel }: { $label } { $score }; Verstoß { $count } in { $window }, Stufe { $step } ({ $decision ->
        [warn] gewarnt
        [observe] nur beobachtet, still
       *[late] zu spät zum Warnen
    })
violation-action = ; { $action }: { $result }
modlog-violation = ; Verstoß { $count } in { $window }, Stufe { $step }

digest-head = 📋 Profanity-Watch-Bericht, { $from } bis { $until }
digest-summary = { $violations ->
        [one] Ein Verstoß
       *[other] { $violations } Verstöße
    } von { $people ->
        [one] einer Person
       *[other] { $people } Personen
    }:
digest-none = Keine Verstöße. 🎉
digest-person = • { $user } ({ $community }): { $count }, meist { $label }, bis Stufe { $step }{ $actions ->
        [0] {""}
        [one] , eine Maßnahme
       *[other] , { $actions } Maßnahmen
    }{ $jar ->
        [none] {""}
       *[other] , Fluchkasse { $jar }
    }

## Moderationsmaßnahmen

action-mute = Stummschalten
action-unmute = Stummschaltung aufheben
action-disconnect = Trennen
action-timeout = Timeout
action-done = erledigt
action-skipped-off = übersprungen: Maßnahmen sind ausgeschaltet
action-skipped-observe = übersprungen: nur beobachten
action-already-muted = so gelassen: jemand anderes hatte die Person schon stummgeschaltet
action-not-connected = fehlgeschlagen: der Bot war nicht mit Fluxer verbunden
action-audit-reason = Profanity Watch: Eskalationsstufe { $step }
action-not-allowed = nicht erlaubt: Der Bot braucht { $permission } und eine Rolle über dieser Person
action-failed = fehlgeschlagen: { $error }
action-for = { $action } für { $duration }

## Wenn der Bot nicht sprechen darf

no-speak = { $user }, { $text }

## Zeitspannen

value-on = an
value-off = aus
spoken-ms = { $shown } Millisekunden
spoken-s = { $n ->
        [one] eine Sekunde
       *[other] { $shown } Sekunden
    }
spoken-min = { $n ->
        [one] eine Minute
       *[other] { $shown } Minuten
    }
spoken-h = { $n ->
        [one] eine Stunde
       *[other] { $shown } Stunden
    }
spoken-d = { $n ->
        [one] einen Tag
       *[other] { $shown } Tage
    }
dur-unlimited = unbegrenzt
dur-ms = { $shown } ms
dur-s = { $shown } s
dur-min = { $shown } min
dur-h = { $shown } h
dur-d = { $n ->
        [one] { $shown } Tag
       *[other] { $shown } Tage
    }

## Der eigene Status des Bots

presence-paused = Pausiert
presence-nobody = Noch niemand verfolgt
presence-watching = Verfolgt { $count ->
        [one] eine Person
       *[other] { $count } Personen
    }{ $observe ->
        [yes] {" "}(nur beobachten)
       *[no] {""}
    }

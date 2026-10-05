# Text the bot writes in chat and in direct messages.
# Mentions (<@id>, <#id>) and pre-formatted numbers arrive as ready-made strings.

## Chat commands

cmd-help = **Profanity watch** · prefix `{ $prefix }` or mention me
    Anyone: `{ $prefix } status` · `{ $prefix } list` · `{ $prefix } jar [@user]` · `{ $prefix } help`
    Admins (Manage community, Administrator or an admin role):
    `{ $prefix } add @user…` / `{ $prefix } remove @user…`: follow these people into voice in this community
    `{ $prefix } pause` / `{ $prefix } resume`: stop or restart following here
    `{ $prefix } observe on|off`: score and log only, never warn
    `{ $prefix } set threshold 0.6 [@user]`: lower is stricter; with @user only for that person
    `{ $prefix } set strikes 2` · `set window 20s` · `set audience offender|tracked|channel` · `set language de`
    `{ $prefix } set <setting> <value> [@user]` · `{ $prefix } reset <setting|all> [@user]`
    `{ $prefix } modlog #channel` / `{ $prefix } modlog off`: post every flagged sentence there{ $audio ->
        [yes] {" "}(with its audio)
       *[no] {""}
    }
cmd-help-ui = Everything else is in the web UI: { $url }
cmd-help-no-ui = Everything else is in the web UI.

cmd-dm-only = I only take commands in a community channel, where I can check who is an admin.
cmd-roles-loading = I am still loading this community's roles; try again in a few seconds.
cmd-denied = Only community admins (Manage community, Administrator or an admin role) can change that.
cmd-unknown = Unknown command `{ $name }`. Try `{ $prefix } help`.
cmd-store-failed = Changed, but saving it failed: { $error }
cmd-usage = Usage: `{ $prefix } { $usage }`
cmd-no-users = Name at least one user: a mention like @name, or a numeric user ID.
cmd-too-many-users = Expected at most one user after the value.
cmd-unknown-setting = Unknown setting `{ $name }`. For example: threshold, strikes, window, audience, language.
cmd-web-only = { $setting } is edited in the web UI.
cmd-no-channel = Name a channel like #mod-log.
cmd-channel-unknown = I can't see that channel in this community.
cmd-channel-not-text = That channel can't hold messages.
cmd-missing-permissions = ⚠️ I'm missing { $permissions } there; give me those or posts will fail.

cmd-list-empty = Nobody is tracked in this community. An admin can add someone with `{ $prefix } add @user`.
cmd-list-head = **Tracked in this community** ({ $count }){ $paused ->
        [yes] {" "}· paused
       *[no] {""}
    }
cmd-list-everywhere = • { $user } (from the bot owner, in every community)
cmd-list-person = • { $user }
cmd-list-note-threshold = {" "}· threshold { $threshold }

cmd-status-head = **Status in this community** (ID { $guild })
cmd-status-paused = ⏸️ Paused
cmd-status-following = Following { $count ->
        [one] one person
       *[other] { $count } people
    }{ $people ->
        [none] {""}
       *[other] : { $people }
    }
cmd-status-mode-observe = Mode: observe only (silent)
cmd-status-mode-warn = Mode: warns when it hears something
cmd-status-detection = Threshold { $threshold } · { $strikes ->
        [one] one strike
       *[other] { $strikes } strikes
    } in { $window } · heard by { $audience ->
        [offender] only the person who said it
        [tracked] tracked people only
       *[channel] everyone in the channel
    }
cmd-status-modlog-off = Mod log: off
cmd-status-modlog = Mod log: { $channel }{ $audio ->
        [yes] {" "}(with audio)
       *[no] {" "}(text only)
    }
cmd-status-room = In voice: { $channel }{ $people ->
        [none] {""}
       *[other] , listening to { $people }
    }
cmd-status-model = Model: { $ready ->
        [yes] ready
       *[no] not answering (restart the bot)
    }{ $device ->
        [none] {""}
       *[other] {" "}({ $device })
    }

cmd-jar-off = The swear jar is switched off here.
cmd-jar-person = { $user } has { $count } in the swear jar.
cmd-jar-empty = The swear jar is empty. 🎉
cmd-jar-head = **Swear jar**
cmd-jar-line = { $rank }. { $user }: { $count }

cmd-add-self = I can't watch myself.
cmd-add-done = Now following { $people } in this community's voice channels.
cmd-add-already = Already tracked: { $people }.
cmd-remove-done = Stopped following { $people } here.
cmd-remove-everywhere = { $people } { $count ->
        [one] is
       *[other] are
    } tracked in every community by the bot owner; only the owner can change that (web UI, global settings).
cmd-remove-missing = Not tracked here: { $people }.
cmd-pause = Paused in this community: I'll leave voice and stop listening. `{ $prefix } resume` to continue.
cmd-resume = Resumed: following tracked people again.
cmd-observe-on = Observe only: I'll score and log, but never warn or act.
cmd-observe-off = Observe only is off: I'll warn again.
cmd-set-community = { $setting } set to { $value } for this community.
cmd-set-person = { $setting } set to { $value } for { $user }.
cmd-reset-community = { $setting } is back to the default for this community.
cmd-reset-person = { $setting } is back to the default for { $user }.
cmd-reset-all-community = Every setting of this community is back to the default.
cmd-reset-all-person = Every setting of { $user } is back to the default.
cmd-modlog-off = Mod log off.
cmd-modlog-set = Mod log set to { $channel }: every flagged sentence is posted there{ $audio ->
        [yes] {" "}with its audio.
       *[no] {" "}(text only; audio in the mod log is switched off).
    }

## Mod log, direct messages and the summary report

modlog-flagged = 🔊 Flagged { $user } in { $channel }: { $labels } · { $seconds } s · { $language } → { $decision ->
        [warn] warned
        [observe] observed (silent)
        [late] too late to warn
        [strike] strike { $strike } of { $of }
       *[other] logged
    }
modlog-chat-flagged = 💬 Flagged { $user } in { $channel }: { $found } → { $decision ->
        [warn] warned
        [observe] observed (silent)
        [strike] strike { $strike } of { $of }
       *[other] logged
    }
    > { $quote }
violation-chat = 💬 **{ $user }** in { $community } / { $channel } wrote { $found }; violation { $count } in { $window }, step { $step } ({ $decision ->
        [warn] warned
       *[observe] observe only, silent
    })
    > { $quote }
chat-warning = please watch your language.
chat-delete-reason = Profanity Watch: a word from the word list
modlog-label-score = { $label } **{ $score }** (bar { $bar })
upload-failed = (audio upload failed: { $error })

violation = 🔊 **{ $user }** in { $community } / { $channel }: { $label } { $score }; violation { $count } in { $window }, step { $step } ({ $decision ->
        [warn] warned
        [observe] observe only, silent
       *[late] too late to warn
    })
violation-action = ; { $action }: { $result }
modlog-violation = ; violation { $count } in { $window }, step { $step }

digest-head = 📋 Profanity watch report, { $from } to { $until }
digest-summary = { $violations ->
        [one] One violation
       *[other] { $violations } violations
    } by { $people ->
        [one] one person
       *[other] { $people } people
    }:
digest-none = No violations. 🎉
digest-person = • { $user } ({ $community }): { $count }, mostly { $label }, up to step { $step }{ $actions ->
        [0] {""}
        [one] , one action
       *[other] , { $actions } actions
    }{ $jar ->
        [none] {""}
       *[other] , swear jar { $jar }
    }

## Moderation actions

action-mute = mute
action-unmute = unmute
action-disconnect = disconnect
action-timeout = time-out
action-done = done
action-skipped-off = skipped: actions are switched off
action-skipped-observe = skipped: observe only
action-already-muted = left as it was: someone else had muted them already
action-not-connected = failed: the bot was not connected to Fluxer
action-audit-reason = Profanity Watch: escalation step { $step }
action-not-allowed = not allowed: the bot needs { $permission } and a role above this person
action-failed = failed: { $error }
action-for = { $action } for { $duration }

## When the bot may not speak

no-speak = { $user }, { $text }

## Lengths of time

value-on = on
value-off = off
spoken-ms = { $shown } milliseconds
spoken-s = { $n ->
        [one] one second
       *[other] { $shown } seconds
    }
spoken-min = { $n ->
        [one] one minute
       *[other] { $shown } minutes
    }
spoken-h = { $n ->
        [one] one hour
       *[other] { $shown } hours
    }
spoken-d = { $n ->
        [one] one day
       *[other] { $shown } days
    }
dur-unlimited = unlimited
dur-ms = { $shown } ms
dur-s = { $shown } s
dur-min = { $shown } min
dur-h = { $shown } h
dur-d = { $n ->
        [one] { $shown } day
       *[other] { $shown } days
    }

## The bot's custom status

presence-paused = Paused
presence-nobody = Nobody to watch yet
presence-watching = Watching { $count ->
        [one] one person
       *[other] { $count } people
    }{ $observe ->
        [yes] {" "}(observe only)
       *[no] {""}
    }

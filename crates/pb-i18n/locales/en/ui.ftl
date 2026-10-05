# The web UI.

ui-ago-now = just now
ui-ago-s = { $n } s ago
ui-ago-min = { $n } min ago
ui-ago-h = { $n } h ago
ui-ago-d = { $n ->
        [one] yesterday
       *[other] { $n } days ago
    }

ui-station-recording = Speaking
ui-station-cut = Cut
ui-station-queued = Queued
ui-station-model = Model
ui-station-verdict = Verdict
ui-station-decision = Decision

ui-decision-clear = Clear
ui-decision-invalid = Invalid score
ui-decision-untracked = No longer tracked
ui-decision-strike = Strike { $strike } of { $of }
ui-decision-warn = Warned (step { $step })
ui-decision-observe = Observed (step { $step })
ui-decision-late = Too late to warn (step { $step })

ui-nav-live = Live
ui-nav-voice-lines = Voice lines
ui-nav-reports = Reports
ui-nav-audit = Audit
ui-nav-system = System
ui-nav-communities = Communities
ui-nav-logout = Log out

ui-offline = Reconnecting…
ui-auth-expired = Your login ended.
ui-log-in-again = Log in again

ui-wall-title = Live
ui-wall-empty = Nobody is being listened to right now. The bot joins a call as soon as a tracked person is in one.
ui-wall-violations = Latest violations
ui-wall-no-violations = No violations yet.
ui-lag = { $ms } to a verdict
ui-not-in-voice = Not in a voice channel
ui-in-channel = In { $channel }
ui-log-in = Log in with Fluxer

ui-on = On
ui-off = Off
ui-none = None
ui-save = Save
ui-use-inherited = Use inherited
ui-inherited = from { $from }
ui-owner-only = bot owner only
ui-duration = A duration like 20s, 5m or 1h
ui-duration-or-unlimited = A duration like 20s, 5m or 1h, or "unlimited"
ui-voice-default = Default voice
ui-voice-by-language = The language's voice
ui-esc-from = From violation
ui-esc-action = Action
ui-esc-duration = Duration
ui-esc-owner = Tell the owner
ui-esc-help = Each step applies from that many violations within the window. Fill in the empty row to add a step; clear "From violation" to remove one.
ui-saved = Saved.
ui-unchanged = Nothing changed.
ui-not-allowed = You may not change this.

ui-not-found = There is nothing here.
ui-tab-overview = Overview
ui-tab-settings = Settings
ui-tab-live = Live
ui-tab-history = History
ui-tab-evidence = Recordings
ui-paused = Paused
ui-unavailable = Unavailable
ui-everywhere = Tracked everywhere
ui-not-tracked = Not tracked
ui-tracked = Tracked
ui-muted = muted
ui-deafened = deafened (hears no warnings)
ui-calls = Calls
ui-joins-paused = The bot was removed from voice here several times in a short while (by a moderator, or another copy of the bot?). It joins again in { $seconds } s and waits a little longer after each further removal. To keep it out of this community, pause it here.
ui-joins-resume = Join again now
ui-joins-resumed = The bot joins here again.
ui-no-calls = Nobody is in a call.
ui-bot-listens = the bot listens
ui-bot-cannot-speak = the bot may not speak
ui-encrypted = end-to-end encrypted
ui-tracked-people = Tracked people
ui-nobody-tracked = Nobody is tracked here yet. Use “Track someone” below (a name, user ID or @mention), or the chat command add, for example !pb add @name.
ui-track = Track
ui-untrack = Stop tracking
ui-track-someone = Track someone
ui-user-id-or-mention = A name, user ID or @mention
ui-track-help = Paste a user ID (or a mention copied from Fluxer). The bot listens to tracked people whenever they are in a call it can join.
ui-violations = Violations
ui-when = When
ui-who = Who
ui-decision = Decision
ui-older = Older
ui-jar = Swear jar
ui-jar-empty = The swear jar is empty.
ui-jar-reset = Empty
ui-digest = Report
ui-digest-help = The report sums up the violations since the last one and is sent to the bot owner (daily or weekly, see Reporting).
ui-digest-send = Send the report now
ui-now = Now
ui-today = Today
ui-in-window = In the window
ui-in-window-value = { $n } (in { $window })
ui-next-step = Step of the next violation
ui-observe-only = Only observing (no warnings)
ui-said-and-done = Said and done
ui-nothing-yet = Nothing yet.
ui-conveyor = Sentences
ui-no-sentences = No sentences yet.
ui-no-evidence = No recordings. Which sentences keep their recording is set under Settings → Recording.
ui-failed = failed
ui-dropped-short = too little speech
ui-dropped-echo = over the bot's own voice
ui-play-warning = Warned
ui-play-strike = Strike notice
ui-play-action = Announced an action
ui-play-greeting = Greeted
ui-play-say = Said
ui-say-now = Say now
ui-say-placeholder = What the bot should say
ui-their-language = Their language
ui-say = Say
ui-say-help = The bot says it in the call this person is in, to the audience set under Warning: a text in a language, a clip from the library, or a Say preset from the voice lines.
ui-days = The last two weeks
ui-day = Day
ui-sentences = Sentences
ui-flagged = Flagged
ui-speech = Speech
ui-length = Length
ui-scores = Scores
ui-delete-recording = Delete the recording
ui-built-in = built in
ui-preview = Preview
ui-remove = Remove
ui-add-text = Add the text
ui-add-clip = Add the clip
ui-clip-removed = (removed clip)
ui-vl-help = What the bot says. A line has clips and texts per language; the bot picks a clip in the language it needs, else speaks the text, else asks the next place up (person, community, global, built in). In a text, { "{" }name{ "}" } is the person's name; also { "{" }label{ "}" } (what was heard), { "{" }count{ "}" } (violations so far), { "{" }step{ "}" }, { "{" }strikes{ "}" }, { "{" }duration{ "}" } (of a mute or time-out), { "{" }server{ "}" } and { "{" }channel{ "}" }.
ui-vl-warning = Warning
ui-vl-any-type = any type
ui-vl-any-step = every step
ui-vl-step = step { $n }
ui-vl-greeting = Greeting
ui-vl-kind-action = Action notices
ui-vl-kind-say = Say presets
ui-vl-strike = Strike notice
ui-vl-action-any = Action notice (any action)
ui-vl-action = Action notice: { $action }
ui-vl-say = Say preset “{ $name }”
ui-vl-say-new = A new Say preset
ui-vl-name = How the name is said
ui-vl-text-placeholder = Text (use {"{"}name{"}"} for the name)
ui-vl-step-placeholder = Step (empty = every step)
ui-vl-preset-placeholder = Preset name (for Say)
ui-vl-add-line = Add a line
ui-clips = Clips
ui-clip-name = Name
ui-clip-no-speech = No speech (any language)
ui-clip-transcript = Transcript (optional)
ui-clip-heard = sounds like { $lang }
ui-clip-sounds-like = sounds like { $label } ({ $score })
ui-no-clips = No clips yet. Upload an audio file or record one.
ui-upload = Upload
ui-record = Record a clip
ui-record-stop = Stop and upload
ui-record-uploading = Uploading…
ui-record-hint = Recording… press again to stop.
ui-all-kinds = Everything
ui-filter = Show
ui-all-communities = Every community
ui-recording-kept = recording kept
ui-heard-language = heard: { $lang }
audit-by-bot = The bot
audit-by-file = A settings file edit
audit-by-old-bot = The old bot
audit-globally = globally
audit-in = in { $community }
audit-for = for { $person } in { $community }
audit-set = { $by } set { $setting } { $where } to { $value }
audit-clear = { $by } reset { $setting } { $where }
audit-track = { $by } started tracking { $person } in { $community }
audit-untrack = { $by } stopped tracking { $person } in { $community }
audit-voice-line = { $by } changed the voice line “{ $line }” { $where }
audit-imported = Old bot: { $by } set { $setting } to { $value }
audit-action = { $action } for { $person } in { $community }: { $result }
audit-jar-reset = { $by } emptied the swear jar of { $person } in { $community }
audit-jar-baseline = The swear jar of { $person } in { $community } starts at { $count } (from the old bot)
audit-recording-deleted = { $by } deleted a recording
audit-clip-saved = { $by } saved the clip “{ $name }”
audit-clip-removed = { $by } removed a clip
audit-voice-saved = { $by } saved the voice “{ $name }”
audit-voice-removed = { $by } removed the voice { $voice }
audit-login = { $name } logged in
audit-msg-modlog = Mod-log post
audit-msg-dm = Message to the owner
audit-msg-digest = Report
audit-message-sent = { $what } sent
audit-message-failed = { $what } could not be sent: { $error }
audit-import = Imported from { $from }: { $sentences } sentences, { $recordings } recordings, { $clips } clips, { $settings } settings
audit-log-repaired = The event log was repaired: { $bytes } cut after event { $seq }
audit-started = The bot started (version { $version })
audit-stopped = The bot stopped
audit-stopped-unclean = The bot stopped before every sentence was scored
audit-unknown = { $kind } (written by a newer version)
audit-group-settings = Settings
audit-group-actions = Moderation actions
audit-group-jar = Swear jar
audit-group-library = Clips and recordings
audit-group-logins = Logins
audit-group-messages = Messages
audit-group-bot = The bot
ui-status = Status
ui-version = Version
ui-microphones = Microphones
ui-models = Models
ui-ready = ready
ui-queues = Work queues
ui-waiting = Waiting
ui-done = Done
ui-oldest = Oldest
ui-parts = Parts of the bot
ui-restarts = Restarts
ui-part-running = running
ui-part-restarting = starting again
ui-part-not-answering = not answering
ui-part-stopped = stopped
ui-part-failed = failed (the bot restarts)
ui-part-moderation = Decisions
ui-part-undo = Lifting mutes
ui-part-digest = Summary reports
ui-part-threads = Model threads
ui-part-views = Live pages
ui-part-gateway = Fluxer connection
ui-part-system = This page
ui-storage = Storage
ui-log = Event log
ui-files = Files
ui-free = Free
ui-index-behind = Index behind
ui-fluxer-no-token = no bot token
ui-fluxer-connecting = connecting
ui-fluxer-ready = online as { $bot }
ui-fluxer-reconnecting = reconnecting ({ $error })
ui-fluxer-no-voice = this Fluxer instance has voice turned off
ui-fluxer-rejected = the token was rejected
ui-fluxer-secrets = Fluxer and secrets
ui-secrets-help = Replacing a secret needs a login from the last 15 minutes. The bot logs in again with a new token at once.
ui-client-secret-help = From the same application in Fluxer (User Settings → Applications). Logins to this web UI use it.
ui-replace = Replace
ui-reconnect = Reconnect to Fluxer
ui-reload-settings = Read the settings and voice files again
ui-clip-added = Added the clip “{ $name }”.
ui-clip-removed-notice = The clip was removed from the library. Voice lines that used it skip it.
ui-clip-unreadable = That file could not be used as a clip: { $error }
ui-digest-not-sent = The report could not be delivered (see below).
ui-digest-sent = The report was sent.
ui-jar-emptied = The swear jar was emptied.
ui-no-longer-tracking = No longer tracking { $name }.
ui-no-such-clip = That clip is not in the library.
ui-not-a-line = That is not a voice line.
ui-not-a-user = That is not a user ID.
ui-now-tracking = Now tracking { $name }.
ui-reconnected = Connected to Fluxer again.
ui-recording-deleted = The recording was deleted.
ui-reloaded = The settings and voice files were read again.
ui-said = Said.
ui-say-empty = Write what the bot should say.
ui-token-replaced = The new token works; the bot is online.
ui-upload-empty = Choose a file to upload.
ui-upload-failed = The upload broke off. Please try again.
ui-secret-from-env = Set in the bot's environment (PB_BOT_TOKEN / PB_CLIENT_SECRET, for example a podman secret); change it there.

perm-view-channel = View channel
perm-send-messages = Send messages
perm-attach-files = Attach files
perm-add-reactions = Add reactions
perm-read-message-history = Read message history
perm-connect = Connect to voice
perm-speak = Speak
perm-mute-members = Mute members
perm-move-members = Move members
perm-moderate-members = Time out members
perm-manage-guild = Manage community
perm-administrator = Administrator
ui-permissions = What the bot may do here
ui-perm-ok = all it needs
ui-perm-missing = missing: { $missing }
ui-perm-modlog = mod log #{ $channel }
ui-perm-actions = moderation actions (they are on)
ui-invite = + Invite the bot
ui-invite-help = Inviting adds the bot to a community on Fluxer. Whoever opens the link needs Manage community there: Fluxer asks which community and shows the permissions the bot asks for. The bot then appears in the sidebar; it joins a call only when someone tracked there is in one.
ui-clip-not-yours = Only who added this clip, or the bot's owner, can change or remove it.
ui-clip-added-by = added by { $name }
ui-track-the-bot = The bot does not track itself.
ui-tracked-everywhere = { $name } is tracked in every community; the owner stops that on their page or on the System page.
ui-digest-last-sent = The last report reached the owner (it covered the time up to { $when }).
ui-digest-failed = The last report could not be delivered: { $error }. Fluxer refuses direct messages when the owner does not accept them from bots; “Send the report now” tries again.
ui-logout-everywhere = Log out on all devices
ui-logout-everywhere-help = Log out on every device (when a login may have been taken over).
ui-permissions-help = Missing permissions are given to the bot's role in Fluxer's community settings (Roles), or for one channel in its permissions.
ui-say-own-text = Text
ui-voices = Voices
ui-voices-help = Piper voices for text-to-speech. For another language, put a voice's two files (.onnx and .onnx.json, from the Piper voices collection) into the voices folder of the bot's data (/data/voices in the container) and read the files again.
ui-record-needs-https = Recording needs the web UI over HTTPS (or on localhost); upload a file instead.
ui-record-refused = The browser did not allow the microphone.
ui-record-upload-failed = The recording could not be uploaded
ui-pause-here = Pause here
ui-resume-here = Resume here
err-not-connected = The bot is not connected to Fluxer right now.
err-not-in-call = They are not in a call.
err-bot-not-in-call = The bot is not in their call.
err-said-too-late = It waited too long and was not said.
err-said-not-spoken = The bot may not speak in that call, so it was not said.
err-said-nothing = There was nothing to say: no clip, text or voice for their language.
err-said-failed = It could not be said: { $error }
err-no-such-sentence = That sentence does not exist.
err-no-recording = That sentence has no recording.
err-no-such-voice = That voice is not in the library (any more).
err-voice-no-cloning = This speech model cannot make voices from samples.
err-voice-needs-transcript = This speech model needs to know what is said in the sample: type it in.
err-voice-no-model = The speech model { $model } does not run (its files are not installed, or it failed to start; see the System page).
err-voice = The voice could not be made: { $error }
err-log-halted = The event log is not writing (see System), so nothing was changed.
err-render = The speech could not be made: { $error }
err-render-no-voice = No installed voice speaks { $lang }.
err-render-clip-missing = A clip of this voice line is missing from the data directory.
err-fluxer = Fluxer said: { $error }
err-bad-instance = That is not a Fluxer instance address: { $error }
err-store = The data could not be read or written: { $error }
err-unknown-host = This address is not one of the bot's web UI addresses. Open it by IP address, then add this name under System → Web UI address or Extra host names.
err-bad-ui-address = the web UI address setting is not a usable address
err-no-login-code = Fluxer sent no login code back
ui-fluxer-stopped = stopped for good: { $error } (the bot needs an update or a new setup)
ui-model-classifier = Classifier (Roblox voice safety)
ui-model-voice-activity = Voice activity (Silero)
ui-model-speech = Text to speech (Piper)
ui-model-not-answering = not answering: restart the bot
ui-model-no-voices = no voice installed
ui-queue-scoring = Sentences to score
ui-queue-speech = Speech to make
ui-log-halted = The event log stopped writing: { $error }. Nothing is recorded until it writes again: free disk space, then try again.
ui-log-retry = Try writing again
ui-log-writing-again = The event log writes again.
ui-index-problem = The search index cannot catch up ({ $error }); it keeps trying. The pages may miss the newest events meanwhile.
ui-index-skipped = { $count ->
    [one] One event in the log was written by a newer version of the bot and is left out of the pages.
   *[other] { $count } events in the log were written by a newer version of the bot and are left out of the pages.
}
ui-bot-joining = the bot is joining
ui-bot-retrying = joining failed; the bot tries again shortly
ui-bot-leaving = the bot is leaving
ui-source = Source code (AGPL-3.0)

## Pausing the whole bot
ui-paused-everywhere = Paused everywhere
ui-paused-everywhere-banner = The bot is paused everywhere: it listens to nobody and says nothing until the bot owner switches it back on.
ui-paused-everywhere-where = Switch it back on (System)
ui-pause-everywhere-title = The bot is running
ui-pause-everywhere-help = Pausing stops it everywhere at once: it leaves every call, listens to nobody and says nothing until you switch it back on. Settings and tracked people stay as they are.
ui-pause-everywhere = Pause everywhere
ui-resume-everywhere-help = The bot listens to nobody and says nothing. Pauses of single communities and people stay in place for when it runs again.
ui-resume-everywhere = Resume everywhere

## Inviting
ui-invite-title = Invite the bot
ui-invite-what = What inviting does
ui-invite-not-connected = The bot is not connected to Fluxer yet, so it cannot make the invite link or see its communities (the bot owner sees why on the System page).
ui-invite-permissions = The link asks for what the bot needs: see channels and write in them (for the mod log), join voice channels and speak. While moderation actions are on in any community, it also asks to mute, move and time out members.
ui-invite-open = Open in Fluxer
ui-invite-no-link = The link appears once the bot has its token and has reached its Fluxer instance.
ui-reauthorize-title = Communities where the bot lacks permissions
ui-reauthorize-help = Authorising again in Fluxer gives the bot's role what is missing (whoever does it needs Manage community there). Permissions taken away in one channel are given back in that channel's settings.
ui-reauthorize = Authorise again
ui-copy = Copy
ui-copied = Copied

## Confirmations
ui-cancel = Cancel
ui-back = Back
ui-confirm-untrack = Stop tracking { $name }?
ui-confirm-untrack-what = The bot stops listening to { $name } in { $community } and leaves their call unless it follows someone else there. What it kept about them (history, recordings, swear jar) stays; tracking them again carries on from there.
ui-confirm-jar = Empty { $name }'s swear jar?
ui-confirm-jar-what = { $count ->
        [one] It holds one violation in { $community }.
       *[other] It holds { $count } violations in { $community }.
    } It starts again at 0; the history of what was said stays.
ui-jar-reset-button = Empty the jar
ui-confirm-recording = Delete this recording?
ui-confirm-recording-what = The recording of { $name } in { $community } from { $when } is deleted for good and cannot be brought back. The sentence, its scores and the decision stay in the history.
ui-confirm-clip = Remove the clip “{ $name }”?
ui-confirm-clip-what = These voice lines use it. They skip it from now on; a line left without clips or texts falls back to the next place up (community, global, built in).
ui-confirm-clip-button = Remove the clip
ui-confirm-clip-elsewhere = { $count ->
        [one] one voice line in a community you do not manage
       *[other] { $count } voice lines in communities you do not manage
    }

## Settings forms
ui-help = What this does
ui-advanced = Advanced
ui-advanced-help = Settings that rarely need changing: how speech is cut into sentences, voices and speech rate, and the bot's own plumbing.
ui-reset-settings = Reset every setting here
ui-reset-settings-help = { $count ->
        [one] One setting is set here.
       *[other] { $count } settings are set here.
    } Resetting makes them all inherited again.
ui-settings-reset = { $count ->
        [one] One setting was reset.
       *[other] { $count } settings were reset.
    }
ui-confirm-reset = Reset every setting here?
ui-confirm-reset-what = These settings, set { $where }, are removed and take their values from { $from } again:
ui-confirm-reset-from-defaults = the settings file or the built-in defaults
ui-confirm-reset-from-global = the global settings
ui-confirm-reset-from-community = the community's settings
ui-confirm-reset-nothing = Nothing is set here that you may reset.
ui-confirm-reset-kept = Pauses and the System settings (the Fluxer instance, the web UI address) stay as they are.
ui-works-with = Works together with “{ $setting }”, which is set { $where }:
ui-where-per-community = per community
ui-where-globally = globally, on the System page
ui-where-per-person = per person
ui-not-set = not set

## Say now, previews
ui-say-clip = Clip
ui-say-preset = Preset
ui-say-play = Play
ui-say-language = Language
ui-say-bot-not-in-call = The bot is not in their call: it joins the calls of tracked people (unless they or the community are paused).
ui-preview-language = The language of the preview
ui-preview-as-bot = As the bot would say it

## First steps, clip names
ui-no-communities = The bot is in no community yet.
ui-first-invite = The bot is in no community yet. Invite it to a Fluxer community first; whoever has Manage community there can add it.
ui-first-track = Nobody is tracked yet. Open a community and track someone: the bot listens to them whenever they are in a call it can join.
ui-clip-name-empty = A clip needs a name.
ui-record-default-name = Recording { $when }

## Chat commands
ui-chat-commands = Chat commands
ui-chat-commands-how = Fluxer has no slash commands and no autocomplete: a command is an ordinary message in a text channel of the community, starting with { $prefix } or a mention of the bot, which answers in that channel.
ui-chat-commands-off = Chat commands are switched off (System → Chat commands).

## Lists of communities, roles and people
ui-list-empty = None yet.
ui-list-add = Add
ui-list-add-community = A community's name or ID
ui-list-add-role = A role's name or ID
ui-list-add-empty = Type a name or paste an ID.
ui-list-added = { $name } added.
ui-list-added-unknown = { $name } added (the bot has not seen this ID yet).
ui-list-already = { $name } is already on the list.
ui-list-removed = { $name } removed.
ui-list-not-there = { $name } is not on the list.
ui-list-no-match = Nothing called “{ $name }” is known. Paste its ID instead.
ui-list-several = Several match “{ $name }”: { $matches }. Type more of the name or paste the ID.
ui-confirm-list-remove = Remove { $name } from “{ $setting }”?
ui-list-remove-what = { $name } is taken off the list.
ui-list-remove-guild-allowlist = The bot stops working in { $name }: it leaves its calls there and does not answer its commands.
ui-list-remove-last-community = It is the last community on the list: with the list empty, the bot works in every community it is in.
ui-list-remove-tracked-everywhere = The bot no longer follows { $name } in every community; where a community tracks them itself, it still does.
ui-list-remove-admin-user-ids = { $name } is no longer an owner of the bot; their logins lose the owner's rights.
ui-list-remove-admin-role-ids = People with the role { $name } no longer manage the bot (unless they manage the community anyway).
ui-track-everywhere = Track in every community
ui-untrack-everywhere = Stop tracking in every community
ui-everywhere-hint = People tracked in every community are taken off that list on their own page or on the System page.

## Pausing, said where
ui-paused-done = Paused { $where }: the bot leaves voice there, listens to nobody and says nothing.
ui-resumed-done = Resumed { $where }: the bot follows the tracked people there again.
ui-in-every-community = in every community

## Names of the bot's parts, people by id
ui-part-recorder = Writing the event log
ui-part-enforcer = Actions and messages
audit-by-id = someone (ID { $id })

## Voice lines: where a line comes from
ui-vl-own = Own line
ui-vl-uses = uses “{ $line }” ({ $from })

## What a save did
ui-saved-what = Saved: { $what }.

## More confirmations
ui-confirm-line-clear = Remove the own clips and texts of “{ $line }”?
ui-confirm-line-clear-what = The line loses { $clips ->
        [one] its clip
       *[other] its { $clips } clips
    } and { $texts ->
        [one] its text
       *[other] its { $texts } texts
    } { $where }. The bot then says what the line says further up (or the built-in text).
ui-confirm-setting-clear = Reset “{ $setting }”?
ui-confirm-setting-clear-what = It goes back to { $value } ({ $from }).
ui-confirm-setting-clear-reconnect = The bot connects to Fluxer again with it.
ui-confirm-logout-all = Log out on all devices?
ui-confirm-logout-all-what = Every login of yours ends, on every device, this one too. Use this when a login may have been taken over.

## Settings pages
ui-nav-settings = Settings
ui-settings-global-title = Settings for every community
ui-settings-intro-global = These apply in every community. A community, or a person in one, can have its own value; that then wins there.
ui-settings-intro-server = Settings for { $community }. What is not changed here follows the settings for every community.
ui-settings-set-here = { $count ->
        [one] one changed here
       *[other] { $count } changed here
    }
ui-settings-moved = The settings for every community have their own pages:

## A section's form
ui-save-changes = Save changes
ui-section-refused = Not saved: { $what } (see below).
ui-back-to-inherited = Back to the inherited value.

## Settings that do nothing at the moment
ui-nr-strikes = Only used with more than one strike (Detection).
ui-nr-digest = Only used while the summary report is on.
ui-nr-digest-weekly = Only used with a weekly summary report.
ui-nr-actions = Only used while moderation actions are on (Escalation).
ui-nr-modlog = Only used once a mod-log channel is chosen.

## Where a setting's value comes from
ui-default = Default
ui-from-file = From the settings file
ui-same-as-global = Same as for every community
ui-same-as-community = Same as in { $community }
ui-changed-default = Changed · default: { $value }
ui-changed-file = Changed · settings file: { $value }
ui-changed-community = Changed for { $community } · every community: { $value }
ui-changed-person = Changed for { $person } · { $community }: { $value }
ui-reset-to-default = Back to the default
ui-use-global-value = Use the value for every community
ui-use-community-value = Use { $community }'s value
ui-changed-in = Changed in { $count ->
        [one] one community:
       *[other] { $count } communities:
    }
ui-settings-intro-person = Settings for { $person } in { $community }. What is not changed here follows { $community }'s settings.
ui-steps = { $count ->
        [one] one step
       *[other] { $count } steps
    }

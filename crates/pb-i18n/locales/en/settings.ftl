# Names and help texts of settings, sections, detection types and choices (web UI, chat replies, docs).
# Ids are "setting-" plus the setting key with "_" written as "-".

## Sections

section-tracking = Tracking
section-detection = Detection
section-warning = Warning
section-escalation = Escalation
section-greeting = Greeting
section-reporting = Reporting
section-recording = Recording
section-commands = Chat commands
section-system = System

## Where a value comes from

source-builtin = built-in default
source-file = settings file
source-global = global
source-server = this community
source-person = this person
docs-scopes = can be set: { $scopes }
docs-who = changed by { $who }
docs-default = default: { $value }
scope-global = Global
scope-server = Community
scope-person = Person
who-admins = community admins and the bot owner
who-owner = only the bot owner
apply-live = applies at once
apply-reconnect = applies after the bot reconnects

## Detection types

label-privacy_asking_for_pii = Asking for personal info
label-discriminatory = Discriminatory
label-harassment = Harassment
label-sexual_content = Sexual content
label-illegal_and_regulated_content = Illegal and regulated
label-dating_and_romantic_content = Dating and romance
label-profanity = Profanity
label-disruptive_audio = Disruptive audio

setting-label-enabled = Warn about { $label }
    .help = Count sentences the model flags as { $label }.
setting-label-threshold = Threshold for { $label }
    .help = How sure the model must be (0 to 1, lower is stricter). Empty: the general threshold.

## Tracking

setting-paused = Paused
    .help = Leave voice and stop listening (for a whole community, or for one person).
setting-guild-allowlist = Only these communities
    .help = Community IDs the bot works in. Empty: every community it is in.
setting-tracked-everywhere = Tracked in every community
    .help = User IDs followed in every community the bot shares with them.
setting-allow-e2ee-downgrade = Join end-to-end encrypted calls
    .help = A bot joining an end-to-end encrypted call switches encryption off for everyone in it.
setting-join-settle = Join delay
    .help = Wait this long before joining, so people hopping between channels don't make the bot churn.
setting-leave-grace = Leave delay
    .help = Stay this long after the last tracked person left.

## Detection

setting-threshold = General threshold
    .help = How sure the model must be before a sentence counts (0 to 1, lower is stricter).
setting-strikes = Strikes before a warning
    .help = Flagged sentences needed within the strike window before the bot reacts.
setting-strike-window = Strike window
    .help = Flagged sentences older than this no longer count as strikes. Unlimited: they always count.
setting-end-silence = Pause that ends a sentence
    .help = Shorter reacts faster, but cuts sentences more often.
setting-max-sentence = Longest sentence
    .help = Long speech is cut at the quietest point after this long.
setting-min-voiced = Shortest speech scored
    .help = Less speech than this in a sentence is not sent to the model.
setting-max-reaction-delay = Latest warning
    .help = A sentence scored later than this after it ended is logged, but not warned about. Unlimited: always warn.

## Warning

setting-observe-only = Observe only (silent)
    .help = Score and log, but never warn or take an action.
setting-audience = Who hears the warning
    .help = Only the person who said it, every tracked person in the call, or everyone in the channel.
setting-volume-db = Warning volume
    .help = Louder or quieter than the recording, in decibels.
setting-voice-language = Spoken language
    .help = The language warnings are spoken in. Auto: the language the model heard.
setting-fallback-languages = Fallback languages
    .help = Tried in this order when there is no voice line in the spoken language.
setting-tts-voices = Text-to-speech voices
    .help = Which voice speaks each language.
setting-speech-rate = Speech rate
    .help = 1 is normal; 1.5 is half again as fast.
setting-no-speak-policy = Without the Speak permission
    .help = Write the warning into the voice channel's chat, or only log it.
setting-strike-notice = Announce strikes
    .help = Say "strike 1 of 3" before the warning itself.
setting-announce-actions = Announce actions
    .help = Say what the bot does (mute, disconnect, time-out) when it does it.

## Escalation

setting-violation-window = Count violations over
    .help = Violations within this time decide the escalation step. Unlimited: every violation counts.
setting-escalation = Escalation steps
    .help = From the nth violation: who is told and which action runs. The step also picks the voice line.
setting-actions-enabled = Allow moderation actions
    .help = Mute, disconnect or time out people as the escalation steps say. Off: warn and tell only.

## Greeting

setting-greet-enabled = Greeting
    .help = Say hello when the bot and this person meet in a call.

## Reporting

setting-modlog-channel = Mod log channel
    .help = Text channel that gets a post for every flagged sentence. Empty: off.
setting-modlog-audio = Audio in the mod log
    .help = Attach the flagged sentence as a recording to mod log posts.
setting-owner-dm-audio = Recordings in messages to the bot owner
    .help = Attach the sentence to the direct message an escalation step sends the bot owner.
setting-digest = Summary report
    .help = A direct message to the bot owner with a summary per person.
setting-digest-time = Report time
    .help = When the summary report is sent.
setting-digest-weekday = Report day (weekly)
    .help = Which day the weekly report is sent.
setting-timezone = Time zone
    .help = IANA name, like Europe/Berlin.
setting-jar-enabled = Swear jar
    .help = Count violations per person (the jar command and the web UI).
setting-chat-language = Chat language
    .help = The language of the bot's chat replies, mod log posts and reports.

## Recording

setting-recordings = Recordings
    .help = Which sentences of tracked people are kept as recordings (each can be deleted later).
setting-admins-play-audio = Community admins may play recordings
    .help = Off: only the bot owner can listen to recordings.

## Chat commands

setting-commands-enabled = Chat commands
    .help = Answer commands like "!pb status" in text channels.
setting-command-prefix = Command prefix
    .help = Text that starts a command. Mentions of the bot always work.
setting-admin-user-ids = Extra bot owners
    .help = These users may do everything the bot owner can, in every community.
setting-admin-role-ids = Admin roles
    .help = Members of these roles may control the bot in that community.

## System

setting-instance = Fluxer instance
    .help = The Fluxer API address, like https://api.fluxer.app or a self-hosted instance.
setting-ui-url = Web UI address
    .help = How this page is opened when not by IP address, like http://botbox.lan:8790.
setting-allowed-hosts = Extra host names
    .help = Other names this page may be opened by.
setting-cpu-threads = CPU threads for the model
    .help = How many threads the classifier uses on the CPU (the GPU ignores this).
setting-tts-threads = CPU threads for speech
    .help = How many threads text-to-speech uses.

## Choices

choice-audience-offender = Only the person who said it
choice-audience-tracked = Tracked people
choice-audience-channel = Everyone in the channel
choice-no-speak-policy-text = Write it in the chat
choice-no-speak-policy-log = Only log it
choice-digest-off = Off
choice-digest-daily = Daily
choice-digest-weekly = Weekly
choice-weekday-monday = Monday
choice-weekday-tuesday = Tuesday
choice-weekday-wednesday = Wednesday
choice-weekday-thursday = Thursday
choice-weekday-friday = Friday
choice-weekday-saturday = Saturday
choice-weekday-sunday = Sunday
choice-recordings-off = None
choice-recordings-flagged = Flagged sentences
choice-recordings-all = Every sentence
choice-step-action-none = None
choice-step-action-mute = Mute
choice-step-action-disconnect = Disconnect
choice-step-action-timeout = Time-out
choice-voice-language-auto = Auto (the language heard)

## Why a value was refused

err-setting = { $setting }: { $problem }
err-unknown-setting = There is no setting called “{ $name }”.
err-scope = { $setting } cannot be set { $scope ->
        [global] globally
        [server] for a community
       *[person] for a person
    }.
err-owner-only = Only the bot owner can change { $setting }.
err-not-probability = { $value } is not between 0 and 1
err-below-one = must be at least 1
err-not-whole = “{ $value }” is not a whole number
err-too-large = “{ $value }” is too large
err-not-duration = “{ $value }” is not a length of time (like 20s, 5m, 2h or 1d)
err-negative = “{ $value }” is negative
err-not-positive = must be longer than zero
err-below-frame = must be at least 32 ms (one audio frame)
err-not-finite = must be a finite number
err-not-above-zero = must be above zero
err-not-time-of-day = “{ $value }” is not a time of day (like 09:00)
err-unknown-tz = unknown time zone “{ $value }” (use a name like Europe/Berlin)
err-not-origin = “{ $value }” is not an http(s) address
err-origin-with-path = “{ $value }” must be just the address, like http://192.168.1.50:8790 (no path)
err-not-host = “{ $value }” is not a host name
err-prefix-spaces = the prefix may not contain spaces
err-not-lang = “{ $value }” is not a language tag (like de or en-US)
err-not-id = “{ $value }” is not an ID
err-not-choice = “{ $value }” must be one of: { $choices }
err-no-steps = escalation needs at least one step
err-step-order = step { $step }: steps must start at increasing violation counts (1, 2, 3 …)
err-timeout-too-long = step { $step }: Fluxer allows time-outs of at most 365.25 days
err-step = step { $step }: { $problem }
err-not-number = “{ $value }” is not a number
err-not-switch = must be on or off
err-not-text = must be text
err-not-list = must be a list
err-unknown-field = unknown field “{ $value }”

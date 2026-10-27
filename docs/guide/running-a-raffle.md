# Running a raffle

A walkthrough of one evening, from setting up to erasing data afterwards. It assumes a running Rafi. To deploy one,
see [Operations](../operations/overview.md). For the draw itself in detail, see [The draw](the-draw.md).

## The evening at a glance

```mermaid
flowchart LR
    A["Before<br/>create the raffle,<br/>add prizes, rehearse"] --> B["Doors open<br/>publish, open registration,<br/>show the projector"]
    B --> C["Start the draw<br/>freeze the list"]
    C --> D["Draw each prize<br/>Draw → Claimed<br/>or Not here / Declined"]
    D --> E["Finish<br/>thanks screen,<br/>export winners"]
    E --> F["After<br/>mark prizes delivered;<br/>data is erased on the date"]
```

Your screens:

| Screen | Where | Shown to |
|---|---|---|
| **Admin** | `/admin` | You. Keep it on a laptop or phone, never on the projector. |
| **Projector screen** | `/s/<slug>` ("Projector screen" button) | The room. It never shows emails or admin buttons. |
| **Participant page** | `/j/<CODE>` (the QR code) | Each attendee, on their phone |
| **Verification page** | `/v/<slug>` ("Verification page" button) | Anyone, once the list is frozen |

## The raffle's states

The admin shows a state badge and the next steps. Each state allows different things:

```mermaid
stateDiagram-v2
    state "Draft" as Draft
    state "Published" as Ready
    state "Registration open" as Open
    state "Registration closed" as Closed
    state "Ready to draw" as Frozen
    state "Drawing" as Drawing
    state "Finished" as Finished
    state "Data erased" as Purged

    [*] --> Draft
    Draft --> Ready: Publish the intro screen
    Ready --> Draft: Unpublish
    Ready --> Open: Open registration
    Open --> Closed: Close registration
    Closed --> Open: Reopen registration
    Open --> Frozen: Freeze list & start the draw
    Closed --> Frozen: Freeze list & start the draw
    Frozen --> Drawing: first Draw
    Drawing --> Finished: Finish the raffle
    Finished --> Purged: Erase personal data now,<br/>or the winner hold passes
    Draft --> Purged: expires
    Ready --> Purged: expires
    Open --> Purged: expires
    Closed --> Purged: expires
```

A raffle that was never frozen **expires** when its erasure date passes, whatever its state: its entrants are
deleted outright and it becomes Data erased.

| State | Prizes and entrants editable | Phones can join | Draws |
|---|---|---|---|
| Draft, Published | yes | no | no |
| Registration open | yes | **yes** | no |
| Registration closed | yes | no | no |
| Ready to draw, Drawing | no (only withdraw an item) | no | **yes** |
| Finished, Data erased | no | no | no |

## Before the event

0. **The first time:** read the service's terms (`/terms`), which include the data processing agreement. Your
   entrants' data is processed on your behalf; the terms say how, and you accept them by using the service. In a
   Clerk organization, **managers** (a custom `org:manager` role, or the `org:raffles:manage` permission, set up
   in Clerk) create, configure and delete raffles; every **member** runs them: registration, the draw, people,
   prizes, exports, erasure; an **admin** can also remove the organization's data.
1. **Create a raffle** in `/admin`: give it a title and the **event date** (today, unless you set it up early; at
   most a year ahead). Entrants' personal data is erased a set number of days after it, and the join form tells
   them the date, so the date and the period lock once a real entrant has joined. Fake entrants don't lock
   anything; someone you add by hand, or your own phone, does. Remove the real entrants (People tab, before the
   freeze) to unlock.
2. **Settings:** the description for the intro screen, the organizer (named as data controller in the rules and
   privacy notice) and a contact email, both needed before registration can open; optionally a separate privacy
   contact (a DPO), your own privacy text or a link to your own privacy notice, shown after the generated notice;
   the **retention period** (how many days after the event non-winners are erased: the service's default, 7,
   unless you choose otherwise, at most 30); and any **additional rules**, one paragraph per line. Put a higher age
   limit there (the service's minimum is in the standard rules, and the join form says the rules may ask for
   more). Save them as **your defaults** so the next raffle starts with them.
3. **Prizes:** add each prize with a quantity and, optionally, a sponsor. Each copy is drawn on its own, in any
   order you like during the draw.
4. **Rehearse.** Turn on **Test mode** in settings. A TEST badge shows on every screen, and the People tab offers
   **Add 25 fake entrants**, with generated names like "Brave Otter". Run through the draw on the projector.

> Rehearse on a copy: **Duplicate** (the raffle's ⋯ menu) copies prizes and settings but not entrants, dates the
> copy today (set its event date), and a raffle drawn in test mode stays a rehearsal: its verification page says
> **TEST MODE** for good. To rehearse on the raffle you'll run live, switch test mode off before the freeze: that
> removes every fake entrant. A raffle that isn't in test mode never freezes with fakes in it.
>
> **Postponed?** The event date is locked once a real entrant has joined, and the data is erased on the date they
> were told. Before the freeze, remove the real entrants to unlock the date, or duplicate the raffle for the new
> date and have people join again. After the freeze, the only option is a new raffle.

## When doors open

1. **Publish the intro screen.** Open the **Projector screen** on the venue's display, in fullscreen (which also
   keeps the screen awake). It shows the title, the prizes and "Registration opens soon".
2. **Open registration.** The projector now shows a QR code, the short join URL and code, a live entrant count and
   recent names, plus "No smartphone? Come to the organizer!".
3. Attendees scan and join with their email and a display name, which is optional: whoever leaves it blank gets a
   generated one, like "Brave Otter" (see [For participants](for-participants.md)).
4. While people join, use the **People** tab to:
   - **add someone without a smartphone** by name (email optional)
   - **hide** or **rename** a display name on screen with one tap (a troll, a typo). New names reach the projector
     after a 5 s delay.
   - **remove** duplicates
   - search by name, email or claim code

You can close and reopen registration as often as you like before the draw.

## Starting the draw

Press **Freeze list & start the draw** when everyone is in. It asks first, because this step is final:

- Registration closes for good. Latecomers can't be added.
- The entrant list is frozen into the **manifest** and published as a hash and a list of anonymous IDs. The
  **Verification page** link appears.
- Prizes are locked. From now on an item can only be **withdrawn**.
- Nobody leaves the list until the raffle is finished (or its erasure date passes): the People tab can still
  **hide** or **rename** a name, and a drawn winner who doesn't want their prize is **Declined**. Asked to erase
  someone's data during the event? Hide their name now, and erase their personal data once the raffle is finished.
- The raffle is handed to the raffle engine.

If handing over fails (for example, the engine is briefly unreachable), the admin offers **Retry handing the list
to the raffle engine**. That picks up where it stopped. If nobody has entered, the freeze is undone.

## Drawing

Open the **Draw** tab. For each prize, in any order:

1. Press **Draw**. The projector shows "Locking in public randomness · drand round R" and a countdown of about 6 s,
   then the winner's name.
2. The admin shows the winner's **claim code**. The winner's phone shows **YOU WON!** with the same code.
3. When they come up and the codes match, press **Claimed**. If they're not in the room, press **Not here**. If
   they don't want it, press **Declined**. With either of those, they're out and the prize goes back to be drawn
   again. Claimed means present with a valid code, not that the prize is in their hands: a voucher you email later
   is **Mark delivered**, afterwards.

[The draw](the-draw.md) covers every case, including what happens when drand can't be reached.

## Finishing

Press **Finish the raffle** when the prizes are handed out (or when you want to stop with some left). The projector
and phones show **Thanks for playing!** with the winners and a link to the verification page. Then:

- **Export winners (CSV)** from the ⋯ menu: entrant ID, name, email, source, claim code, outcome, prize, delivered.
- **End live updates** when the room empties. Phones and the projector stop following live, and the results stay on
  the pages. This also happens automatically a day after the finish.

## After the event

Personal data is erased on the dates entrants were shown, whatever you do; the raffle's page says which. You can
only make it sooner:

- **Mark delivered** (People tab) for each prize that reached its winner, or **All prizes delivered** (⋯ menu) once
  nothing is still on its way. Winners' names and emails are kept until then so you can deliver, and at most for
  the winner hold (90 days past the erasure date: a prize you can't deliver by then needs the winner's address
  elsewhere, exported in time); everyone else's go at the end of the erasure date (7 days after the event unless
  you chose otherwise). Once the date has passed, marking delivered erases that winner on the next hourly run.
- **Erase one entrant's personal data** (People tab) when someone asks.
- **Erase personal data now** (⋯ menu) once you no longer need names and emails. Names, emails and claim codes go,
  and the anonymous draw log stays verifiable. This can't be undone.
- **Export winners (CSV)** while their details are still here: the export is logged, and the copy is yours to keep
  and erase under your own rules.
- **Delete raffle** removes it entirely, entrants and all, **until the first draw is announced**: it is the way out
  of a freeze by accident (a "fixed" list means everyone joins again under a new code). Once a draw has happened,
  the raffle can't be deleted: its record is what makes the draw verifiable, and "Erase personal data now" is the
  way to be rid of what's personal in it. Raffles whose data is erased are hidden from your list by default.
- A raffle you never finished is erased on the date all the same, and asked to finish once the hold passes; a raffle
  you never froze simply expires: its entrants are deleted.

See [Privacy and abuse](../explanation/privacy-and-abuse.md). Leaving Rafi? **Your data** at the bottom of the
raffle list removes your defaults and your name from your raffles, and erases their personal data. In an
organization, an admin does this, for every co-host's raffles.

## Checklist

- [ ] Prizes entered with the right quantities and sponsors
- [ ] Event date right, organizer, contact email and additional rules filled in (registration won't open without
      an organizer and a contact)
- [ ] Rehearsed on a duplicate in test mode, with the projector
- [ ] The projector screen in fullscreen, readable from the back of the room
- [ ] The venue wifi or mobile data reaches the server over HTTPS
- [ ] Someone ready to add people without a phone
- [ ] Nobody deploys while the raffle is live (see [Upgrades](../operations/upgrades.md))

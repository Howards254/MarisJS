# MarisJS benchmark tasks

Seven small UI-building tasks. Each describes what to build in plain language — the way a teammate would ask. None leak implementation details.

A task is "done" when a real browser renders the described behavior and every acceptance criterion passes.

---

## Task 1 — Bookmark list

Build a page that shows a list of bookmarks. Each bookmark has a title and a URL. There's a form at the top to add a new one — two text inputs and an Add button. The list updates immediately when a bookmark is added. Next to each bookmark is a Delete button that removes it from the list.

**Acceptance criteria:**
- The page starts with zero bookmarks and shows "No bookmarks yet."
- Typing a title and URL then clicking Add appends a new row.
- The inputs clear after adding.
- Clicking Delete removes that row.
- When the last item is deleted, "No bookmarks yet." reappears.
- Adding a bookmark with an empty title or URL does nothing (no blank rows appear).

---

## Task 2 — Password signup form

Build a signup form with three fields: email, password, and confirm password. There's a Submit button. As the user types, live validation messages appear below the fields:

- **Email**: must contain an `@` and a `.` after it. Show "Enter a valid email" if it doesn't.
- **Password**: must be at least 8 characters. Show "At least 8 characters" until the minimum is reached.
- **Confirm password**: must match the password field. Show "Passwords don't match" when they differ.

The Submit button stays disabled until all three fields pass validation. When the user clicks Submit while the form is valid, replace the form with a success message: "Account created for [email]."

**Acceptance criteria:**
- Submit button is disabled on page load (all fields empty).
- Typing "a" in email shows the email error immediately.
- Typing "a@b.c" in email clears the email error.
- Typing "1234567" in password shows the length error; "12345678" clears it.
- Typing a different value in confirm password shows the mismatch error; matching clears it.
- Submit button enables only when all three pass.
- Clicking Submit replaces the form with the success message containing the correct email.

---

## Task 3 — Shopping cart with total

Build a shopping cart for a fruit stand. Show four products — apples ($1.50), bananas ($0.75), oranges ($2.00), and grapes ($3.50). Each product has a quantity: a minus button, the current quantity number, and a plus button. Below the products, show the order total, updated live.

Quantities can't go below zero. The plus button increments by 1. The minus button decrements by 1 and is disabled when quantity is zero.

**Acceptance criteria:**
- All quantities start at 0.
- Clicking + on apples changes its quantity to 1 and total to $1.50.
- Clicking + twice more makes apples 3 and total $4.50.
- Clicking - on apples decrements: 3 → 2 → 1 → 0. At 0 the minus button is disabled.
- Adding 2 bananas ($0.75 × 2 = $1.50) and 1 orange ($2.00) while apples is 3 ($4.50) shows total $8.00.
- Total displays with exactly two decimal places (e.g. $1.50, not $1.5).

---

## Task 4 — Temperature converter

Build a temperature unit converter. There are two labeled inputs — Celsius and Fahrenheit. Typing in one field instantly updates the other. The conversion formulas are:

- °F = (°C × 9/5) + 32
- °C = (°F − 32) × 5/9

When you type in Celsius, Fahrenheit updates. When you type in Fahrenheit, Celsius updates. The conversion should be rounded to one decimal place.

**Acceptance criteria:**
- Both fields start empty.
- Typing "0" in Celsius shows "32.0" in Fahrenheit.
- Typing "100" in Celsius shows "212.0" in Fahrenheit.
- Typing "32" in Fahrenheit shows "0.0" in Celsius.
- Typing "212" in Fahrenheit shows "100.0" in Celsius.
- Typing "-40" in either field shows "-40.0" in the other.
- Clearing one field clears the other.

---

## Task 5 — Notification badge (parent/child)

Build a notification system with two parts. At the top of the page, a bell icon (use the text "🔔") with a badge showing the total unread count. Below it, a list of notification cards. Each card shows a message text and a "Mark read" button. When you click "Mark read," that card disappears from the visible list and the bell badge count drops by one.

The notifications are pre-populated with these three items:

1. "Your order has shipped"
2. "New comment on your post"
3. "Password changed successfully"

The badge should show the count of still-unread notifications. An "All read" message appears when the list is empty.

**Acceptance criteria:**
- Badge shows "3" on page load.
- All three notifications are visible.
- Clicking "Mark read" on the first notification removes it from the list and the badge changes to "2".
- Clicking "Mark read" on both remaining items empties the list, shows "All read" text, and the badge shows "0".
- The bell icon and badge persist at the top even when the list is empty.

---

## Task 6 — Countdown timer

Build a countdown timer. There's a number input (seconds) and a Start button. When you enter a number and click Start, the timer counts down to zero one second at a time, displaying the remaining seconds. When it reaches zero, it stops and shows "Time's up!". While the timer is running, the Start button changes to a Stop button, and the input is disabled. Clicking Stop pauses the countdown; the button changes back to Start and the input re-enables. Clicking Start again resumes from where it stopped.

**Acceptance criteria:**
- Input starts empty, button says "Start".
- Typing "3" and clicking Start disables the input, button becomes "Stop", display shows "3".
- After 1 second: display shows "2". After 2 more seconds: display shows "0" and "Time's up!" replaces the number.
- Type "5", click Start, immediately click Stop: display shows "5" frozen, button is "Start", input is enabled.
- Click Start again: countdown resumes from 5.
- The page behaves correctly if you type "0" and click Start (shows "Time's up!" immediately).

---

## Task 7 — Tabs with lazy content

Build a tabbed interface with three tabs: "Profile", "Settings", and "Activity". Only one tab's content is visible at a time. The active tab has a distinct visual style (darker background or underline — implementer's choice). Each tab shows different content:

- **Profile**: "Profile content for [a hardcoded username like 'alice']"
- **Settings**: "Settings panel"
- **Activity**: "Activity feed"

**Acceptance criteria:**
- "Profile" is selected by default and its content is visible.
- Clicking "Settings" hides Profile content, shows Settings content, and highlights the Settings tab.
- Clicking "Activity" works the same way.
- Clicking "Profile" from another tab switches back correctly.
- Only one tab looks active at any time.

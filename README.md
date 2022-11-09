# Javelin

Touchpad on laptops with large screens has a problem: depending on cursor acceleration it's either too slow or imprecise. So, if it's too slow that reqires several swipes to move cursor from one screen corner to another, and if it's too fast that reqires several swipes because cursor is hard to aim. With that touchpad usage becomes quite distracting and annoying. Let also add that in order to execute a precise cursor movement user must remember where cursor stays otherwise another swipe is required to find it.

I propose a solution which allows user to precisely aim cursor at the target with only two *short* swipes. The first swipe moves cursor fast to "throw" it in an appropriate direction while the second moves it slowly to precisely complete the movement. Also, user don't need to remember where the cursor position is because after a short timeout it moves to the center of active window, so the starting position of movement remains very predictable.

![example](example.svg)

There are some sensible timeouts between motions that makes the cursor behavior intuitive and following DWIM motive [Do What I Mean]. It's perfect for tiling window managers because only one swipe is required to switch focus between windows. Also, it's perfect for reading documents and navigating the Web because it's very easy with it to focus on various elements, for example buttons. Although, it's unsuitable for gaming because with it users must keeps in sync with the current cursor mode and that makes constant cursor movement very erroneous.

*I call it "Javelin" style motion because it's cursor movement remembers projectile movement of famous Javelin anti-tank weapon as well as of a regular javelin. There's also some word-contrast with "arrow" style cursor movement which I like.*

The prototype is built in Rust language and depends on Sway window manager for Linux, which implemets Wayland window protocol. Theoretically it would be possible to build something similar for Xorg window managers as well as for other operating systems however I'm concerned about resourse usage. The Rust programming language was choosen for speed but also because it's just a great language!

It uses the following libraries:

- `clap` argument parser used for timeouts configuration
- `input` [libinput]() bindings for Rust to read cursor movements
- `libc` required for `input` to work
- `signal-hook` to reset pointer speed and other Sway properties on termination signals
- `spin-sleep` used in cursor animation in fast mode
- `swayipc` to alter cursor speed and get window position in Sway

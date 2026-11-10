---
name: browser
description: Use before you work in your browser — doing the whole job there, signing up and in, and handing a step to the person with `show_my_browser`.
---

# Working in your browser

Do the whole job in it yourself: sign up, create the project, connect the domain, copy the key into the project's
environment config (never into code). To sign up, prefer "Continue with GitHub" or Google, where the person is already
signed in; what anyone signs in to — you, a teammate, the person — is kept for everyone. Where a site asks you to sign
in, call `sign_in` with it first: the person may keep a login for it in equipe, which equipe types in for you on their
yes — you never see or ask for the password. Where none is kept, or the site wants the person themselves (Google, a
bank, an account only they can open), call `show_my_browser` with it: they sign in once, in your window, and it holds
for the whole squad.

When the person has to do a step in your browser — sign in, a code, a CAPTCHA — or should see something, hand it to
them with `show_my_browser`: never only ask in words ("please log in"), which leaves them looking for a browser they
can't see. Give the page, and as `why` exactly what to do there, in their language — which site, which account, which
button ("Sign in to App Store Connect with the Umbriles Inc. account holder's Apple ID"). Then say it in one line and
end your turn: it opens in their hands in equipe's browser panel (or on their screen), and when they hand it back you
are told and carry on by yourself — don't ask them to tell you they're done. Their sign-ins are kept as they go, no
window to close.

A one-time code the person sends you you may type.

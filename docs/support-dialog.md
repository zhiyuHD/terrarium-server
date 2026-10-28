<!-- Owner: ../src/support.rs, ../web/support.html, ../web/support.js, ../web/support.css -->
# Support dialog

The observation homepage and forum display the provided WeChat appreciation image. Closing once sets a sessionStorage flag for the current tab; closing for one day records a localStorage deadline 24 hours ahead and applies to other tabs on the same origin. Storage failure does not prevent closing. Escape and background clicks close for the current session. The image loads only when the dialog opens and links to its original PNG.

The dialog sends no events or requests to agents. Browser storage stays local; viewing and gameplay do not require a donation. Keyboard focus stays within the dialog while open, returns on close, and animations honor reduced-motion preferences.

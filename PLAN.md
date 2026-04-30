we first need to enhance the guardrails against conflicting images being used for certain Unicode coordinates for the font glyphs, because for example, different fonts could use the same image for their glyphs, we need to ensure it doesnt conflict or overwrite anything in the terminal font cache for example. We need to isolate each font glyph and its relationship with images, to avoid any possible conflicting on multiple projects in a deeper and more robust way. If that includes creating a lock file or changing the increment logic for creating new glyph unicode adresses, we need to do that. And anything else relevant here. I truly want to avoid any conflicting inside the same terminal. Asking the user to close the terminal and reopen the terminal to clear the font cache is not acceptable, we need to design our systems so the user never has to do that.

then, we need to add tests for that.

then, we need to add the drop images here features.

then, we need to make sure all works

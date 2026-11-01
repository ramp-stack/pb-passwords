# Passphrase word list

`resources/wordlists/words.txt` contains 2,048 distinct lowercase everyday
English words authored for this project. It uses common objects, places,
activities, foods, seasons, and descriptions, including rainy, october, and
simply. It is original project content, not EFF's dictionary. No external
word-list download or runtime network request is needed.

The generator samples each of the three words independently with operating-system
randomness and rejection sampling. Three words from this list provide 33 bits
of word-selection entropy. Optional capitalization, a symbol, and a digit add
independent variation. The three-word format follows the requested product
behavior and is not a claim of universal password-policy compliance.

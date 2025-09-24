# TODO

- Generate manifest so we can see statistics, errors, and all files downloaded and their paths on the filesystem.
  - This could also help us if some assets are missing or not downloaded yet, or if we need to go back and forth between local and remote urls.
- Rewrite html paths in files.
- Proxy support
- Rewrite html paths for absolute urls and also for relative urls that need to end with /index or .html
- Make the manifest hold information about when the file was last downloaded.
- Figure out how to handle query parameters. on pcs they are: scraped/procyclingstats.com/www.procyclingstats.com/index.html?popular=pro_me&s=upcoming-races&category=1.html and scraped/procyclingstats.com/www.procyclingstats.com/badge.php?id=17.html
- Allow using random user agent from a file/list
- Make sure filters work.


## Notes

- Maybe rip our Webp
- Maybe we need UrlMapper to handle url resolution, where we pass it the file or url that a url was found in so it can resolve the relative url relative to that file. How this works would differ though between if it is a local file or a remote url, I think.
- We are logging incorrectly: Saved HTML to: sites/scrapethissite-domains/sites/scrapethissite-domains/www.scrapethissite.com/index.html

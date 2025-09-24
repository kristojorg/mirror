1. Add --continue command
  - Make it work for webp (MAYBE RIP OUT WEBP)
  - Make it check html files DONE
  - Make it check js files also DONE
2. Generate manifest so we can see statistics, errors, and all files downloaded.
  - This could also help us if some assets are missing or not downloaded yet, or if we need to go back and forth between local and remote urls.
3. Generate a log for each run that details what happened in that run, while the manifest details the current state.
4. Download files from external sources into separate folders by domain name, similar to how httrack works.
5. Proxy support
6. Rewrite html paths for absolute urls and also for relative urls that need to end with /index or .html
7. Make the manifest hold information about when the file was last downloaded.
8. Figure out how to handle query parameters. on pcs they are: scraped/procyclingstats.com/www.procyclingstats.com/index.html?popular=pro_me&s=upcoming-races&category=1.html and scraped/procyclingstats.com/www.procyclingstats.com/badge.php?id=17.html
9. Allow using random user agent from a file/list
10. Make sure filters work.


## Notes

- Maybe we need UrlMapper to handle url resolution, where we pass it the file or url that a url was found in so it can resolve the relative url relative to that file. How this works would differ though between if it is a local file or a remote url, I think.
- We are logging incorrectly: Saved HTML to: sites/scrapethissite-domains/sites/scrapethissite-domains/www.scrapethissite.com/index.html

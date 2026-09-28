# The CRAP metric

CRAP combines cyclomatic complexity and test coverage into one number that is
high when code is both hard to understand and poorly tested. Savoia and Evans
introduced the metric in 2007, with implementations for Java (Crap4j) and .NET
(NDepend). `cargo-crap` is the Rust one.

Background: the blog post
[cargo-crap: Finding Untested Complexity in AI-Generated Rust Code](https://minikin.me/blog/cargo-crap)
and the talk [Your AI Code Might Be CRAP! (Here's How To Fix It)](https://www.youtube.com/watch?v=XuMR1pgc6pc).

```text
CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)
```

Properties of the formula:

- A trivial function (CC=1, 100% covered) scores exactly 1.0, the lower bound.
- At 100% coverage the quadratic term collapses and **CRAP equals CC**.
  Matching values in those two columns mean the function is fully covered.
  Tests cap the damage, but the complexity itself remains.
- Above CC ≈ 30 no amount of coverage keeps a function under the default
  threshold of 30, since at full coverage the score is CC itself.

## Prior art and references

- [Savoia, A. & Evans, B. (2007). *The CRAP Metric.*](https://www.artima.com/weblogs/viewpost.jsp?thread=210575)
- [Crap4j](http://www.crap4j.org/), the original Java implementation.
- [dry4go](https://github.com/unclebob/dry4go), the Go duplicate detector
  whose normalize, fingerprint and Jaccard approach `--duplicates` follows.

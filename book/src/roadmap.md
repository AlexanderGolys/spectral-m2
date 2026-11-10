# Roadmap

What is planned next. Released changes are listed in the [changelog](https://github.com/AlexanderGolys/spectral-m2/blob/master/CHANGELOG.md).

## Your own classes, drawn your way

Today a class can shape how it prints only through `net`, which was made for black-and-white terminals: once a class draws its own text layout, nothing can tell which characters were a number or a type, so it cannot be coloured. The plan is a new method a class can implement beside `net` and `tex`, giving one version of the value for each way of drawing maths:

- [ ] a typeset version: TeX plus highlight groups naming the shell's own colours, so it follows every scheme;
- [ ] a Unicode version, laid out in characters, in colour;
- [ ] the plain `net`, as now, for any terminal.

The common library types will provide all three, and clicking an output's icon will step through the versions a value has.

## Smaller items

- [ ] Typeset maths as the default on terminals that can show it, falling back to Unicode elsewhere.
- [ ] Colour typeset formulas without ever affecting their spacing, so a formula never loses its colours.

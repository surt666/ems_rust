1. The logical_data table still have both measured and resampled values and timestamps. It should only be the resampled ones renomed to value/timestamp and the resample-method column is also unneeded
2. The default sample time should be 15 minutes. When we setup a sensor that should be default but 5,15,30 or 60 should be choosable
3. Edit formulas also allow me to edit the hierarchy. Hierarchy can only be edited on company level by an Admin. Node formulas can be edited on the node as an an Admin, wher Admin is the cognito group
4. Edit formulas lacks space between buttons, delete buttons should be red, and the explanations also lack spacing and explanation of acronyms, that I dont know whether are international
5. It seems texts have been hardcoded danish, so verify the i18n files are used
6. Closing of the formula dialog box doesnt work. Verify it is a dialog and not some old framework
7. Verify that we are using straight html/css with grid not flex, and hyperscript or alpine.js instead of pure js whereever posssible
8. The graphs now have more whitespace than data, so make the actual grophs use the space
